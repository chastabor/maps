//! Doorways, corridors and exits: turning the grown areas into a connected
//! cave system.

use crate::AreaKind;
use crate::grid::{Hex, HexGrid};
use crate::growth::{Areas, find, weighted_index};
use crate::ruins::RuinShape;
use crate::tags::{ConnectTag, ExitTag, LayoutTag, Tags};
use rand::Rng;
use rand::seq::SliceRandom;

/// How many cells wide a connection's floor may be, at most.
///
/// The bound that makes a connection a *link* rather than an opening. One cell is the narrowest
/// possible and matches what a doorway has always occupied, so it is where this starts; widening it
/// widens every corridor in the map, which is a look decision rather than a correctness one.
const CONNECTION_WIDTH: usize = 1;
use std::collections::{BTreeMap, HashSet};

/// One link between areas `a` and `b`, and the floor it occupies.
///
/// The single object every room-to-room connection is built from — see
/// `plans/connection-object.md`. A doorway and a corridor are the same thing at different lengths:
/// a doorway is one cell of floor with a leaf drawn across it, a corridor is several with none.
/// Keeping both as one object is what stops the two being decided in different places and
/// disagreeing, which is where every corridor bug has lived.
///
/// **`cells` is keyed on cells, not on area indices, deliberately.** `growth::finalize` and
/// `keep_largest_component` re-map area indices, so anything stored against an `(a, b)` pair cannot
/// be carried across them — that is what made a claimed corridor's pairing unrecoverable and left
/// corridors pointing at areas that had been dropped. Cells are never re-mapped.
#[derive(Clone, Debug)]
pub struct Connection {
    /// The **frontage**: every free cell where these two areas face each other at this place,
    /// anchor first. How wide the link *could* be, not how wide it is.
    ///
    /// Two rooms sitting side by side share a broad front, and claiming all of it does not build a
    /// corridor — it opens the rooms to each other along their whole facing edge and they dissolve
    /// into one space. (Rendered: `plans/wip/whole.png`, where two of three maps lost nearly every
    /// room.) So this is the *budget* the link is cut from, and [`Self::along`] is what it takes.
    pub across: Vec<Hex>,
    /// The floor the link actually **occupies** — a bounded-width run cut from [`Self::across`],
    /// anchor first. This is what gets claimed and what the walls are built from.
    ///
    /// For areas that face each other directly this is one cell deep by construction: a candidate
    /// cell touches *both* areas, so there is nothing between the two borders but it. The bound is
    /// therefore on width, and [`CONNECTION_WIDTH`] sets it.
    pub along: Vec<Hex>,
    pub a: usize,
    pub b: usize,
    /// Whether a door leaf is drawn across it. A corridor is a connection without one.
    pub doored: bool,
}

impl Connection {
    /// The anchor: the cell the leaf spans, and the cell the run was grown from.
    pub fn cell(&self) -> Hex {
        self.along[0]
    }
}

/// An opening to the outside: an `attach` cell inside an area plus a short
/// passage of free cells walking away from the map centre.
#[derive(Clone, Debug)]
pub struct Exit {
    pub area: usize,
    pub attach: Hex,
    pub stub: Vec<Hex>,
}

pub struct Topology {
    pub connections: Vec<Connection>,
    pub exits: Vec<Exit>,
    /// Per-area flag: true if the area was shrunk into a corridor.
    pub is_corridor: Vec<bool>,
    /// Door pairs merged into one wide opening on a short rect wall, as
    /// `(door_i, door_j, pillar_cell)` — see `merged_pillar_pairs`. Computed
    /// once here; `doorway::mouths` unions each pair into one mouth and
    /// `outline::floor_and_narrow` fills the pillar cell so the wide opening is
    /// backed by continuous floor.
    pub merged_doors: Vec<(usize, usize, Hex)>,
}

pub fn build<R: Rng>(
    grid: &HexGrid,
    areas: &mut Areas,
    tags: &Tags,
    s: f64,
    rng: &mut R,
) -> Topology {
    // Fused rooms sharing an edge are one compound; door topology treats each
    // compound as a single node so it gets one door per external neighbour (not
    // one per member), and the seam between members gets none.
    let group = fuse_groups(areas);
    let pairs = candidate_cells_by_pair(grid, areas, &group);
    let edges = cull_edges(pairs.keys().copied().collect(), areas.count(), tags, rng);
    let connections: Vec<Connection> = edges
        .iter()
        .map(|&(ga, gb)| {
            let cands = &pairs[&(ga, gb)];
            // The RNG draw is unchanged and stays first: everything below is derived from the
            // cell it picks, so the stream is untouched.
            let (cell, a, b) = cands[rng.random_range(0..cands.len())];
            // The connection's whole run, not just the cell the leaf spans: the contiguous stretch
            // of candidate cells joining these same two AREAS at this same place. A group pair can
            // touch in several separate spots, so the run is grown outward from the chosen cell
            // rather than taken as every candidate for the pair.
            //
            // Nothing reads this yet — `cell()` still returns the head — so it is inert until the
            // walls are built from it. It is here because it is the input that step 2 needs, and
            // deriving it at the moment the edge is chosen is the whole point of the object: this
            // is where the connection is decided, so this is where its extent belongs.
            let local: HashSet<Hex> = cands
                .iter()
                .filter(|&&(_, x, y)| (x, y) == (a, b))
                .map(|&(h, _, _)| h)
                .collect();
            let mut across = vec![cell];
            let mut frontier = vec![cell];
            while let Some(h) = frontier.pop() {
                for nb in h.neighbors() {
                    if local.contains(&nb) && !across.contains(&nb) {
                        across.push(nb);
                        frontier.push(nb);
                    }
                }
            }
            // The link takes a bounded width from the frontage, grown outward from the anchor a
            // ring at a time so it stays contiguous and centred rather than taking whichever cells
            // the flood happened to reach first.
            let mut along = vec![cell];
            while along.len() < CONNECTION_WIDTH {
                let Some(next) = across
                    .iter()
                    .copied()
                    .filter(|h| !along.contains(h))
                    .filter(|h| along.iter().any(|k| k.distance(*h) == 1))
                    .min_by_key(|h| h.distance(cell))
                else {
                    break;
                };
                along.push(next);
            }
            Connection {
                across,
                along,
                a,
                b,
                doored: true,
            }
        })
        .collect();

    let exits = place_exits(grid, areas, tags, rng);
    let is_corridor = shrink_corridors(areas, &connections, &exits, tags, rng);
    let merged_doors = merged_pillar_pairs(areas, &connections, s);

    Topology {
        connections,
        exits,
        is_corridor,
        merged_doors,
    }
}

/// The dungeon room both doors pierce, if any (the wall a merged opening cuts).
pub(crate) fn shared_dungeon_room(areas: &Areas, a: &Connection, b: &Connection) -> Option<usize> {
    [a.a, a.b]
        .into_iter()
        .find(|&r| areas.kind(r) == AreaKind::Dungeon && (r == b.a || r == b.b))
}

/// Union areas that share a cell edge — only fused areas touch (everyone else
/// keeps a rock gap) — and return each area's compound root. A non-fused area
/// is its own singleton group, so this leaves non-fused maps unchanged.
fn fuse_groups(areas: &Areas) -> Vec<usize> {
    let n = areas.count();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..areas.count() {
        for c in areas.floor_cells(i) {
            for nb in c.neighbors() {
                if let Some(o) = areas.owner_of(nb).filter(|&o| o != i) {
                    let (ri, ro) = (find(&mut parent, i), find(&mut parent, o));
                    if ri != ro {
                        parent[ri] = ro;
                    }
                }
            }
        }
    }
    (0..n).map(|i| find(&mut parent, i)).collect()
}

/// Distance-2 door pairs that share a dungeon room, both on the **same** rect
/// wall, separated by a single unowned rock cell — a lone pillar wedged between
/// the two openings on a short wall. Returned as `(door_i, door_j, pillar)`.
///
/// Merging such a pair into one wide opening (in `doorway::mouths`) needs the
/// pillar cell added to the floor (in `outline::floor_and_narrow`), else it
/// stays a rock nub floating in the opening and the floor outline weaves around
/// it — the two passages cross. Same-edge only: a pair straddling a corner (one
/// on the north wall, one on the east) is not a pinched double door, and carving
/// one opening across the corner folds the outline. A straight distance-2 pair
/// shares exactly one neighbour; a bent one shares two and must stay separate.
pub(crate) fn merged_pillar_pairs(
    areas: &Areas,
    doors: &[Connection],
    s: f64,
) -> Vec<(usize, usize, Hex)> {
    // Which of a rect's four edges a point sits outside (equality is all we
    // compare, so the discriminant value is arbitrary).
    let rect_wall = |cx: f64, cy: f64, hw: f64, hh: f64, p: (f64, f64)| -> u8 {
        let (dx, dy) = ((p.0 - cx) / hw, (p.1 - cy) / hh);
        match (dx.abs() >= dy.abs(), dx >= 0.0, dy >= 0.0) {
            (true, true, _) => 0,
            (true, false, _) => 1,
            (false, _, true) => 2,
            (false, _, false) => 3,
        }
    };
    let mut out = Vec::new();
    for i in 0..doors.len() {
        for j in i + 1..doors.len() {
            if doors[i].cell().distance(doors[j].cell()) != 2 {
                continue;
            }
            let Some(room) = shared_dungeon_room(areas, &doors[i], &doors[j]) else {
                continue;
            };
            let Some(RuinShape::Rect { cx, cy, hw, hh }) = areas.shape(room) else {
                continue;
            };
            if rect_wall(cx, cy, hw, hh, doors[i].cell().center(s))
                != rect_wall(cx, cy, hw, hh, doors[j].cell().center(s))
            {
                continue;
            }
            // Exactly one shared neighbour (the pillar), and it must be rock.
            let mut shared = doors[i]
                .cell()
                .neighbors()
                .into_iter()
                .filter(|n| doors[j].cell().neighbors().contains(n));
            if let (Some(p), None) = (shared.next(), shared.next())
                && areas.owner_of(p).is_none()
            {
                out.push((i, j, p));
            }
        }
    }
    out
}

/// Door candidates grouped by the unordered pair of fusion groups they could join.
/// Each candidate is `(the free cell, one bordering area, the other)`.
type CandidatesByPair = BTreeMap<(usize, usize), Vec<(Hex, usize, usize)>>;

/// Free cells adjacent to two or more areas, grouped by the unordered pair of
/// their fusion **groups**. Same-group adjacencies are interior to a compound
/// (the seam) and contribute nothing. Each candidate keeps the two real
/// bordering areas so the chosen door attaches to an actual room, not a group.
fn candidate_cells_by_pair(grid: &HexGrid, areas: &Areas, group: &[usize]) -> CandidatesByPair {
    let mut by_pair: CandidatesByPair = BTreeMap::new();
    for &h in grid.cells() {
        if areas.owner_of(h).is_some() {
            continue;
        }
        let mut adj: Vec<usize> = h
            .neighbors()
            .iter()
            .filter_map(|n| areas.owner_of(*n))
            .collect();
        adj.sort_unstable();
        adj.dedup();
        for i in 0..adj.len() {
            for j in i + 1..adj.len() {
                let (a, b) = (adj[i], adj[j]);
                let (ga, gb) = (group[a], group[b]);
                if ga == gb {
                    continue;
                }
                by_pair
                    .entry((ga.min(gb), ga.max(gb)))
                    .or_default()
                    .push((h, a, b));
            }
        }
    }
    by_pair
}

/// Cull area-pairs according to the connectivity tag:
/// tree keeps a random spanning tree (no loops); connected breaks one edge of
/// every fully-connected triangle; untagged keeps all pairs.
fn cull_edges<R: Rng>(
    mut edges: Vec<(usize, usize)>,
    n_areas: usize,
    tags: &Tags,
    rng: &mut R,
) -> Vec<(usize, usize)> {
    match tags.connect {
        Some(ConnectTag::Tree) => {
            edges.shuffle(rng);
            let mut parent: Vec<usize> = (0..n_areas).collect();
            fn find(parent: &mut Vec<usize>, x: usize) -> usize {
                if parent[x] != x {
                    parent[x] = find(parent, parent[x]);
                }
                parent[x]
            }
            edges
                .into_iter()
                .filter(|&(a, b)| {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    if ra == rb {
                        false
                    } else {
                        parent[ra] = rb;
                        true
                    }
                })
                .collect()
        }
        Some(ConnectTag::Connected) => {
            let mut alive: HashSet<(usize, usize)> = edges.iter().copied().collect();
            for a in 0..n_areas {
                for b in a + 1..n_areas {
                    for c in b + 1..n_areas {
                        let tri = [(a, b), (b, c), (a, c)];
                        if tri.iter().all(|k| alive.contains(k)) {
                            alive.remove(&tri[rng.random_range(0..3)]);
                        }
                    }
                }
            }
            edges.retain(|e| alive.contains(e));
            edges
        }
        None => edges,
    }
}

fn place_exits<R: Rng>(grid: &HexGrid, areas: &Areas, tags: &Tags, rng: &mut R) -> Vec<Exit> {
    let want = match tags.exits {
        Some(ExitTag::Sealed) => 0,
        Some(ExitTag::Entrance) => 1,
        Some(ExitTag::Passage) => 2,
        Some(ExitTag::Junction) => rng.random_range(3..=4),
        None => match rng.random_range(0..10) {
            0 => 0,
            1..=6 => 1,
            _ => 2,
        },
    };
    let mut exits: Vec<Exit> = Vec::new();
    if want == 0 {
        return exits;
    }

    // Candidate attach cells: area cells with a free outward neighbour,
    // weighted by squared distance from the centre so exits hug the rim.
    let mut cand: Vec<(usize, Hex)> = Vec::new();
    for i in 0..areas.count() {
        for h in areas.floor_cells(i) {
            if !outward_steps(grid, areas, i, h, &[]).is_empty() {
                cand.push((i, h));
            }
        }
    }

    let mut tries = 0;
    while exits.len() < want && tries < 200 && !cand.is_empty() {
        tries += 1;
        let weights: Vec<f64> = cand
            .iter()
            .map(|&(_, h)| {
                let d = h.distance(Hex::ORIGIN) as f64;
                d * d + 1.0
            })
            .collect();
        let k = weighted_index(rng, &weights);
        let (area, attach) = cand[k];
        if exits.iter().any(|e| e.attach.distance(attach) < 4) {
            cand.remove(k);
            continue;
        }

        // Walk outward until the map edge; a stub that gets stuck before the
        // rim would dead-end mid-map, so discard the candidate instead.
        let mut stub: Vec<Hex> = Vec::new();
        let mut cur = attach;
        let at_rim = |h: Hex| grid.edge_distance(h) == Some(0);
        while !at_rim(cur) {
            let steps = outward_steps(grid, areas, area, cur, &stub);
            if steps.is_empty() {
                break;
            }
            cur = steps[rng.random_range(0..steps.len())];
            stub.push(cur);
        }
        if !at_rim(cur) {
            cand.remove(k);
            continue;
        }
        exits.push(Exit { area, attach, stub });
    }
    exits
}

/// Free in-grid neighbours of `cur` strictly nearer the map edge whose own
/// neighbourhood touches no area other than `area` (so exit passages never
/// merge with doors or other chambers).
///
/// Nearness to the **edge** rather than distance from the centre: on a
/// rectangle those differ, and it is the edge a passage leaving the map is
/// aiming for — heading away from the centre of a tall board can run parallel
/// to the near side for a long time without ever arriving.
fn outward_steps(grid: &HexGrid, areas: &Areas, area: usize, cur: Hex, stub: &[Hex]) -> Vec<Hex> {
    let d0 = grid.edge_distance(cur).unwrap_or(i32::MAX);
    cur.neighbors()
        .into_iter()
        .filter(|&n| {
            grid.contains(n)
                && areas.owner_of(n).is_none()
                && !stub.contains(&n)
                && grid.edge_distance(n).is_some_and(|d| d < d0)
                && n.neighbors()
                    .iter()
                    .all(|m| areas.owner_of(*m).is_none_or(|o| o == area))
        })
        .collect()
}

/// Randomly pick areas (preferring many-doored ones; burrow raises the odds)
/// and shrink each to a minimal connected set still touching all its doors
/// and exit attachments.
fn shrink_corridors<R: Rng>(
    areas: &mut Areas,
    doors: &[Connection],
    exits: &[Exit],
    tags: &Tags,
    rng: &mut R,
) -> Vec<bool> {
    let n = areas.count();
    let mut door_cells: Vec<Vec<Hex>> = vec![Vec::new(); n];
    for d in doors {
        door_cells[d.a].push(d.cell());
        door_cells[d.b].push(d.cell());
    }
    let mut keep_cells: Vec<Vec<Hex>> = vec![Vec::new(); n];
    for e in exits {
        keep_cells[e.area].push(e.attach);
    }
    // Fusion-corridor floor is the join to a partner, not part of this area's own shape:
    // shrinking it away would unfuse the pair with no door to replace the seam.
    for (i, keep) in keep_cells.iter_mut().enumerate() {
        keep.extend(areas.floor_cells(i).filter(|&c| areas.is_join(c)));
    }

    let burrow = tags.layout == Some(LayoutTag::Burrow);
    let hub = tags.layout == Some(LayoutTag::Hub);

    let mut is_corridor = vec![false; n];
    for i in 0..n {
        let n_doors = door_cells[i].len();
        // Dungeon rooms are grown as their final shape and must keep it —
        // never shrink one into a winding corridor.
        if n_doors < 2 || (hub && i == 0) || areas.kind(i) == AreaKind::Dungeon {
            continue;
        }
        let mut p = 0.2 + 0.12 * (n_doors as f64 - 2.0);
        if burrow {
            p += 0.35;
        }
        if !rng.random_bool(p.min(0.85)) {
            continue;
        }
        let floor: Vec<Hex> = areas.floor_cells(i).collect();
        let removed = shrink(&floor, &door_cells[i], &keep_cells[i], rng);
        // Marked, not removed: the cells stay in the area's footprint as a record of where
        // it was, and only ownership is released — so they read as rock, and another area
        // may claim them. See `plans/immutable-growth.md`.
        areas.mark_eroded(i, &removed);
        is_corridor[i] = true;
    }
    is_corridor
}

/// Repeatedly remove random cells while the remainder stays connected and
/// every door/keep constraint holds. Converges to a winding width-1 passage.
fn shrink<R: Rng>(cells: &[Hex], doors: &[Hex], keep: &[Hex], rng: &mut R) -> Vec<Hex> {
    let mut remaining: Vec<Hex> = cells.to_vec();
    let mut removed: Vec<Hex> = Vec::new();
    loop {
        let mut order: Vec<usize> = (0..remaining.len()).collect();
        order.shuffle(rng);
        let mut progressed = false;
        for &k in &order {
            let cell = remaining[k];
            if keep.contains(&cell) || remaining.len() <= 1 {
                continue;
            }
            let test: Vec<Hex> = remaining.iter().copied().filter(|&c| c != cell).collect();
            let doors_ok = doors
                .iter()
                .all(|d| d.neighbors().iter().any(|m| test.contains(m)));
            if doors_ok && is_connected(&test) {
                remaining = test;
                removed.push(cell);
                progressed = true;
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    removed
}

fn is_connected(cells: &[Hex]) -> bool {
    if cells.is_empty() {
        return true;
    }
    let set: HashSet<Hex> = cells.iter().copied().collect();
    let mut seen: HashSet<Hex> = HashSet::from([cells[0]]);
    let mut stack = vec![cells[0]];
    while let Some(c) = stack.pop() {
        for m in c.neighbors() {
            if set.contains(&m) && seen.insert(m) {
                stack.push(m);
            }
        }
    }
    seen.len() == cells.len()
}
