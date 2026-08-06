//! The guarantees the shared-pocket rule in `topology` rests on — see the sequencing
//! refactor in `plans/tile-corridor-render.md`.
//!
//! All are properties of the OUTPUT rather than restatements of the placement rule, which is
//! what lets them catch a bug in it:
//!
//! - **Nothing a connection *takes* closes a gap.** A widening candidate is refused unless a cell
//!   of rock survives between this passage and every other one, so the only way two runs can end
//!   up touching is through the cells no connection can give up: their anchors, drawn from the
//!   frontage before any other connection has one. Hence for every touching pair of run cells, at
//!   least one is its connection's anchor — and a stale occupancy set, an unsequenced widening
//!   pass, or a barrier test that looks at claim status instead of occupancy all break exactly
//!   this. (The last one is how the bug got in: the first barrier keyed on `Areas::is_join`, which
//!   an organic-sided passage never sets.)
//! - **Dropping a pathway loses no reachability.** `prune_pockets` removes the claimed passages a
//!   pocket has no room for, each removal checked against the survivors. The external form of
//!   that guarantee is a comparison of two partitions: areas joined by fusion seams and the
//!   surviving connections, against areas joined by fusion seams and *any* passable cell that
//!   borders both. Nothing in the first may be split that the second joins.
//! - **Only a claimed link is wider than its anchor** (the `Connection::along` doc invariant).
//!   An unclaimed link gets exactly one cell of floor (`outline::floor_and_narrow` opens `cell()`
//!   and nothing else), so width on one would be floor no part of the map ever lays — the phantom
//!   width the sequencing refactor removed, pinned here so it cannot creep back.
//!
//! Deliberately NOT "every area is reachable", because two pre-existing conditions break that and
//! neither is this rule's doing — both measured identical before the rule and after:
//! - some maps grow two clusters far enough apart that no free cell ever borders both, and those
//!   have always had two separate cave systems (2 maps of 200 in the fused config);
//! - an area can come out orphaned upstream, with no connection and no seam at all (fused seed 189,
//!   area 8: a Ruin with 18 cells of floor and nothing joining it to anything). Those areas are
//!   excluded from the comparison and counted instead. The rule cannot create one: a drop requires
//!   the two areas joined another way, so an area holding a single connection always keeps it.
//!
//! The anchor property is asserted on FREE-RUN cells only ([`Connection::run`]). An apron cell is
//! room floor the passage crosses to reach a fitted border; two passages entering the same room
//! legitimately cross adjacent room tiles, and rock between those is not something the rule ever
//! promised.
//!
//! `SEEDS` widens the seed range (default 60, matching the other topology-level suites).
//!
//! [`Connection::run`]: maps_core::topology::Connection::run

use maps_core::grid::Hex;
use maps_core::growth::find;
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

const CONFIGS: [&str; 5] = [
    "large,organic,separate",
    "medium,coral,wet,organic,mosaic",
    "large,ruins,dungeon,separate",
    "large,ruins,dungeon,fused",
    "large,chamber,connected,ruins,dungeon,truchet",
];

fn union(p: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(p, a), find(p, b));
    p[ra] = rb;
}

#[test]
fn pocket_invariants() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    // Exercise counters — a clean invariant also means "never ran".
    let (mut conns, mut touches, mut reachable, mut orphans) = (0usize, 0usize, 0usize, 0usize);
    let mut unclaimed = 0usize;
    for tags in CONFIGS {
        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Tags::parse(tags).ok(),
                    ..GenOptions::default()
                },
            );
            let cs = &m.topology.connections;
            let n = m.areas.count();
            conns += cs.len();

            // The seam partition: areas fused into one compound share the compound's doors
            // (`fuse_groups` makes them ONE node to door topology), so a member may carry no
            // connection of its own and reach the map through the seam. Both partitions below
            // start from it.
            let mut base: Vec<usize> = (0..n).collect();
            for i in 0..n {
                for c in m.areas.floor_cells(i) {
                    for nb in c.neighbors() {
                        if let Some(o) = m.areas.owner_of(nb).filter(|&o| o != i) {
                            union(&mut base, i, o);
                        }
                    }
                }
            }
            // An area with no connection and no seam was never offered a pathway at all, so its
            // isolation is upstream of this rule — and the rule cannot produce one, since a drop
            // needs the two areas joined some other way and an area with a single connection
            // therefore always keeps it. Counted, not silently skipped. "Has a seam" is "its
            // seam component has more than one member".
            let seam_root: Vec<usize> = (0..n).map(|i| find(&mut base, i)).collect();
            let mut size = vec![0usize; n];
            for &r in &seam_root {
                size[r] += 1;
            }
            let attached: Vec<bool> = (0..n)
                .map(|i| size[seam_root[i]] > 1 || cs.iter().any(|c| c.a == i || c.b == i))
                .collect();
            orphans += attached.iter().filter(|a| !**a).count();

            // 1. The connections join everything a passable cell could join.
            let mut linked = base.clone();
            for c in cs {
                union(&mut linked, c.a, c.b);
            }
            let mut possible = base;
            for &h in m.grid.cells() {
                // What `candidate_cells_by_pair` saw as free: a cell claimed for a connection is
                // owned now but was free then, and an eroded cell is free now but was the area's
                // then — so a claim counts as passable and erosion does not.
                if !(m.areas.is_join(h) || (m.areas.owner_of(h).is_none() && !m.areas.is_eroded(h)))
                {
                    continue;
                }
                let mut adj: Vec<usize> = h
                    .neighbors()
                    .iter()
                    .filter_map(|&nb| m.areas.owner_of(nb))
                    .collect();
                adj.sort_unstable();
                adj.dedup();
                for w in adj.windows(2) {
                    union(&mut possible, w[0], w[1]);
                }
            }
            let pr: Vec<usize> = (0..n).map(|i| find(&mut possible, i)).collect();
            let lr: Vec<usize> = (0..n).map(|i| find(&mut linked, i)).collect();
            for i in 0..n {
                for j in (i + 1)..n {
                    if !(attached[i] && attached[j] && pr[i] == pr[j]) {
                        continue;
                    }
                    reachable += 1;
                    assert_eq!(
                        lr[i], lr[j],
                        "seed {seed} [{tags}]: areas {i} and {j} could be joined by a passable \
                         cell but nothing joins them — a pathway was dropped that nothing replaced"
                    );
                }
            }

            // 2. Where two runs touch, at least one of the two cells is an anchor.
            let cells: Vec<(usize, Hex, bool)> = cs
                .iter()
                .enumerate()
                .flat_map(|(i, c)| {
                    c.run()
                        .iter()
                        .enumerate()
                        .map(move |(k, &h)| (i, h, k == 0))
                })
                .collect();
            for (p, &(i, x, x_anchor)) in cells.iter().enumerate() {
                for &(j, y, y_anchor) in &cells[p + 1..] {
                    if i == j || x.distance(y) > 1 {
                        continue;
                    }
                    touches += 1;
                    assert!(
                        x_anchor || y_anchor,
                        "seed {seed} [{tags}]: connections {i} and {j} touch at \
                         ({},{})/({},{}), neither an anchor — a widening step closed a gap it \
                         was supposed to refuse",
                        x.q,
                        x.r,
                        y.q,
                        y.r
                    );
                }
            }

            // 3. Only a claimed link is wider than its anchor.
            for (i, c) in cs.iter().enumerate() {
                if c.claimed {
                    continue;
                }
                unclaimed += 1;
                assert_eq!(
                    c.run().len(),
                    1,
                    "seed {seed} [{tags}]: unclaimed connection {i} carries a run wider than \
                     its anchor — floor the outline never lays (it opens only the anchor)"
                );
            }
        }
    }
    println!(
        "{conns} connections ({unclaimed} unclaimed, all anchor-only), {touches} run-cell \
         touches (every one at an anchor), {reachable} passable-reachable area pairs (every one \
         joined), {orphans} areas orphaned upstream and skipped"
    );
    assert!(conns > 1000, "only {conns} connections exercised");
    assert!(
        touches > 100,
        "only {touches} touching run-cell pairs exercised — the anchor property never had a \
         chance to fail"
    );
    assert!(
        reachable > 1000,
        "only {reachable} passable-reachable area pairs exercised — assertion 1 is vacuous"
    );
    assert!(
        unclaimed > 1000,
        "only {unclaimed} unclaimed connections exercised — assertion 3 is vacuous"
    );
}
