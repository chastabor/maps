//! Doorways, corridors and exits: turning the grown areas into a connected
//! cave system.

use crate::AreaKind;
use crate::grid::{Hex, HexGrid};
use crate::growth::{Areas, weighted_index};
use crate::tags::{ConnectTag, ExitTag, LayoutTag, Tags};
use rand::Rng;
use rand::seq::SliceRandom;
use std::collections::{BTreeMap, HashSet};

/// A passable gap cell joining areas `a` and `b`.
#[derive(Clone, Copy, Debug)]
pub struct Door {
    pub cell: Hex,
    pub a: usize,
    pub b: usize,
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
    pub doors: Vec<Door>,
    pub exits: Vec<Exit>,
    /// Per-area flag: true if the area was shrunk into a corridor.
    pub is_corridor: Vec<bool>,
}

pub fn build<R: Rng>(grid: &HexGrid, areas: &mut Areas, tags: &Tags, rng: &mut R) -> Topology {
    // Fused rooms sharing an edge are one compound; door topology treats each
    // compound as a single node so it gets one door per external neighbour (not
    // one per member), and the seam between members gets none.
    let group = fuse_groups(areas);
    let pairs = candidate_cells_by_pair(grid, areas, &group);
    let edges = cull_edges(pairs.keys().copied().collect(), areas.count(), tags, rng);
    let doors: Vec<Door> = edges
        .iter()
        .map(|&(ga, gb)| {
            let cands = &pairs[&(ga, gb)];
            let (cell, a, b) = cands[rng.random_range(0..cands.len())];
            Door { cell, a, b }
        })
        .collect();

    let exits = place_exits(grid, areas, tags, rng);
    let is_corridor = shrink_corridors(areas, &doors, &exits, tags, rng);

    Topology {
        doors,
        exits,
        is_corridor,
    }
}

fn uf_find(parent: &mut [usize], x: usize) -> usize {
    if parent[x] != x {
        parent[x] = uf_find(parent, parent[x]);
    }
    parent[x]
}

/// Union areas that share a cell edge — only fused areas touch (everyone else
/// keeps a rock gap) — and return each area's compound root. A non-fused area
/// is its own singleton group, so this leaves non-fused maps unchanged.
fn fuse_groups(areas: &Areas) -> Vec<usize> {
    let n = areas.count();
    let mut parent: Vec<usize> = (0..n).collect();
    for (i, cells) in areas.cells.iter().enumerate() {
        for c in cells {
            for nb in c.neighbors() {
                if let Some(o) = areas.owner_of(nb).filter(|&o| o != i) {
                    let (ri, ro) = (uf_find(&mut parent, i), uf_find(&mut parent, o));
                    if ri != ro {
                        parent[ri] = ro;
                    }
                }
            }
        }
    }
    (0..n).map(|i| uf_find(&mut parent, i)).collect()
}

/// Free cells adjacent to two or more areas, grouped by the unordered pair of
/// their fusion **groups**. Same-group adjacencies are interior to a compound
/// (the seam) and contribute nothing. Each candidate keeps the two real
/// bordering areas so the chosen door attaches to an actual room, not a group.
fn candidate_cells_by_pair(
    grid: &HexGrid,
    areas: &Areas,
    group: &[usize],
) -> BTreeMap<(usize, usize), Vec<(Hex, usize, usize)>> {
    let mut by_pair: BTreeMap<(usize, usize), Vec<(Hex, usize, usize)>> = BTreeMap::new();
    for &h in grid.cells() {
        if areas.owner_of(h).is_some() {
            continue;
        }
        let mut adj: Vec<usize> = h.neighbors().iter().filter_map(|n| areas.owner_of(*n)).collect();
        adj.sort_unstable();
        adj.dedup();
        for i in 0..adj.len() {
            for j in i + 1..adj.len() {
                let (a, b) = (adj[i], adj[j]);
                let (ga, gb) = (group[a], group[b]);
                if ga == gb {
                    continue;
                }
                by_pair.entry((ga.min(gb), ga.max(gb))).or_default().push((h, a, b));
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

fn place_exits<R: Rng>(
    grid: &HexGrid,
    areas: &Areas,
    tags: &Tags,
    rng: &mut R,
) -> Vec<Exit> {
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
    for (i, area) in areas.cells.iter().enumerate() {
        for &h in area {
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
        while cur.distance(Hex::ORIGIN) < grid.radius {
            let steps = outward_steps(grid, areas, area, cur, &stub);
            if steps.is_empty() {
                break;
            }
            cur = steps[rng.random_range(0..steps.len())];
            stub.push(cur);
        }
        if cur.distance(Hex::ORIGIN) < grid.radius {
            cand.remove(k);
            continue;
        }
        exits.push(Exit { area, attach, stub });
    }
    exits
}

/// Free in-grid neighbours of `cur` strictly further from the centre whose
/// own neighbourhood touches no area other than `area` (so exit passages
/// never merge with doors or other chambers).
fn outward_steps(
    grid: &HexGrid,
    areas: &Areas,
    area: usize,
    cur: Hex,
    stub: &[Hex],
) -> Vec<Hex> {
    let d0 = cur.distance(Hex::ORIGIN);
    cur.neighbors()
        .into_iter()
        .filter(|&n| {
            grid.contains(n)
                && areas.owner_of(n).is_none()
                && !stub.contains(&n)
                && n.distance(Hex::ORIGIN) > d0
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
    doors: &[Door],
    exits: &[Exit],
    tags: &Tags,
    rng: &mut R,
) -> Vec<bool> {
    let n = areas.count();
    let mut door_cells: Vec<Vec<Hex>> = vec![Vec::new(); n];
    for d in doors {
        door_cells[d.a].push(d.cell);
        door_cells[d.b].push(d.cell);
    }
    let mut keep_cells: Vec<Vec<Hex>> = vec![Vec::new(); n];
    for e in exits {
        keep_cells[e.area].push(e.attach);
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
        let removed = shrink(&areas.cells[i], &door_cells[i], &keep_cells[i], rng);
        areas.remove_from_area(i, &removed);
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
