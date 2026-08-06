//! The two guarantees the shared-pocket rule in `topology` rests on — see the sequencing
//! refactor in `plans/tile-corridor-render.md`.
//!
//! Both are properties of the OUTPUT rather than restatements of the placement rule, which is
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
//! - **Dropping a pathway loses no reachability.** `prune_pockets` removes the dungeon-to-dungeon
//!   passages a pocket has no room for, each removal checked against the survivors. The external
//!   form of that guarantee is a comparison of two partitions: areas joined by fusion seams and
//!   the surviving connections, against areas joined by fusion seams and *any* passable cell that
//!   borders both. Nothing in the first may be split that the second joins.
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
//! The anchor property is asserted on FREE-RUN cells only. An apron cell is room floor the passage
//! crosses to reach a fitted border (`Connection::apron_from` bounds them); two passages entering
//! the same room legitimately cross adjacent room tiles, and rock between those is not something
//! the rule ever promised.
//!
//! `SEEDS` widens the seed range (default 60, matching the other topology-level suites).

use maps_core::grid::Hex;
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

const CONFIGS: [&str; 5] = [
    "large,organic,separate",
    "medium,coral,wet,organic,mosaic",
    "large,ruins,dungeon,separate",
    "large,ruins,dungeon,fused",
    "large,chamber,connected,ruins,dungeon,truchet",
];

fn find(p: &mut Vec<usize>, x: usize) -> usize {
    if p[x] != x {
        let r = find(p, p[x]);
        p[x] = r;
    }
    p[x]
}

fn union(p: &mut Vec<usize>, a: usize, b: usize) {
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

            // 1. The connections join everything a passable cell could join.
            //
            // A fused compound is ONE node to door topology (`fuse_groups`), so a member can
            // carry no connection of its own and reach the map through the seam it shares — both
            // partitions start from the seams for that reason.
            let seams = |parent: &mut Vec<usize>| {
                for i in 0..n {
                    for c in m.areas.floor_cells(i) {
                        for nb in c.neighbors() {
                            if let Some(o) = m.areas.owner_of(nb).filter(|&o| o != i) {
                                union(parent, i, o);
                            }
                        }
                    }
                }
            };
            let mut linked: Vec<usize> = (0..n).collect();
            seams(&mut linked);
            for c in cs {
                union(&mut linked, c.a, c.b);
            }
            let mut possible: Vec<usize> = (0..n).collect();
            seams(&mut possible);
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
            // An area with no connection and no seam was never offered a pathway at all, so its
            // isolation is upstream of this rule — and the rule cannot produce one, since a drop
            // needs the two areas joined some other way and an area with a single connection
            // therefore always keeps it. Counted, not silently skipped.
            let attached: Vec<bool> = (0..n)
                .map(|i| {
                    cs.iter().any(|c| c.a == i || c.b == i)
                        || m.areas.floor_cells(i).any(|c| {
                            c.neighbors()
                                .iter()
                                .any(|nb| m.areas.owner_of(*nb).is_some_and(|o| o != i))
                        })
                })
                .collect();
            orphans += attached.iter().filter(|a| !**a).count();

            let pr: Vec<usize> = (0..n).map(|i| find(&mut possible, i)).collect();
            let lr: Vec<usize> = (0..n).map(|i| find(&mut linked, i)).collect();
            let joinable: Vec<(usize, usize)> = (0..n)
                .flat_map(|i| ((i + 1)..n).map(move |j| (i, j)))
                .filter(|&(i, j)| attached[i] && attached[j] && pr[i] == pr[j])
                .collect();
            reachable += joinable.len();
            let lost: Vec<(usize, usize)> = joinable
                .into_iter()
                .filter(|&(i, j)| lr[i] != lr[j])
                .collect();
            assert!(
                lost.is_empty(),
                "seed {seed} [{tags}]: area pairs {lost:?} could be joined by a passable cell but \
                 nothing joins them — a pathway was dropped that nothing replaced"
            );

            // 2. Where two runs touch, at least one of the two cells is an anchor.
            let runs: Vec<&[Hex]> = cs.iter().map(|c| &c.along[..c.apron_from]).collect();
            for i in 0..runs.len() {
                for j in (i + 1)..runs.len() {
                    for &x in runs[i] {
                        for &y in runs[j] {
                            if x.distance(y) > 1 {
                                continue;
                            }
                            touches += 1;
                            assert!(
                                x == cs[i].cell() || y == cs[j].cell(),
                                "seed {seed} [{tags}]: connections {i} and {j} touch at \
                                 ({},{})/({},{}), neither an anchor — a widening step closed a gap \
                                 it was supposed to refuse",
                                x.q,
                                x.r,
                                y.q,
                                y.r
                            );
                        }
                    }
                }
            }
        }
    }
    println!(
        "{conns} connections, {touches} run-cell touches (every one at an anchor), \
         {reachable} passable-reachable area pairs (every one joined), \
         {orphans} areas orphaned upstream and skipped"
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
}
