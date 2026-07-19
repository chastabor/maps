//! Seed-growth: grow N disjoint areas cell by cell, keeping a one-cell gap
//! between areas so that gap cells can later become doorways.

use crate::AreaKind;
use crate::grid::{CellMap, Hex, HexGrid};
use crate::tags::{LayoutTag, ShapeTag, SizeTag, Tags};
use rand::Rng;

/// May the growing area `idx` (of kind `cur_kind`) sit next to the cell owned
/// by `owner_val`? Free cells and its own cells are fine; a foreign area only
/// if the two kinds fuse ([`AreaKind::may_fuse`] — dungeon↔dungeon only).
/// `kinds` holds the kinds of already-grown areas, indexed by area id.
fn neighbor_ok(owner_val: Option<u32>, idx: usize, cur_kind: AreaKind, kinds: &[AreaKind]) -> bool {
    owner_val.is_none_or(|o| o as usize == idx || cur_kind.may_fuse(kinds[o as usize]))
}

/// Extend `frontier` with the still-valid free neighbours of newly added cell
/// `c`: in-grid, unowned, and every one of their own neighbours passes
/// [`neighbor_ok`].
fn extend_frontier(
    frontier: &mut std::collections::BTreeSet<Hex>,
    grid: &HexGrid,
    owner: &CellMap<u32>,
    idx: usize,
    cur_kind: AreaKind,
    kinds: &[AreaKind],
    c: Hex,
) {
    for n in c.neighbors() {
        if grid.contains(n)
            && !owner.contains(n)
            && n.neighbors().iter().all(|m| neighbor_ok(owner.get(*m), idx, cur_kind, kinds))
        {
            frontier.insert(n);
        }
    }
}

/// Numeric parameters resolved from tags for the growth step.
#[derive(Clone, Debug)]
pub struct GrowthParams {
    /// Target cell count for each area (first entry is the hub when present).
    pub sizes: Vec<usize>,
    /// Connectedness preference: candidate weight is `c^gamma` where `c` is
    /// the number of neighbours already in the area. High = rounder blobs,
    /// negative = narrow tendrils.
    pub gamma: f64,
    /// Chaotic shape overrides gamma: prefer cells with exactly 2 connections.
    pub chaotic: bool,
}

pub fn resolve<R: Rng>(tags: &Tags, rng: &mut R) -> GrowthParams {
    let count = match tags.size {
        Some(SizeTag::Small) => rng.random_range(2..=3),
        Some(SizeTag::Medium) => rng.random_range(3..=8),
        Some(SizeTag::Large) => rng.random_range(9..=19),
        None => match rng.random_range(0..20) {
            0 => 2,
            19 => 20,
            _ => rng.random_range(3..=12),
        },
    };

    let gamma = match tags.shape {
        Some(ShapeTag::Cavities) => 6.0,
        Some(ShapeTag::Coral) => -rng.random_range(0.5..2.0),
        _ => rng.random_range(0.5..3.0),
    };
    let chaotic = tags.shape == Some(ShapeTag::Chaotic);

    let sizes: Vec<usize> = match tags.layout {
        Some(LayoutTag::Hub) => {
            let mut v = vec![rng.random_range(60..=79)];
            for _ in 1..count.max(2) {
                v.push(rng.random_range(8..=13));
            }
            v
        }
        Some(LayoutTag::Chamber) => {
            let base = rng.random_range(11..=14) as i64;
            (0..count)
                .map(|_| (base + rng.random_range(-2..=2)).max(5) as usize)
                .collect()
        }
        Some(LayoutTag::Burrow) => (0..count)
            .map(|_| {
                let m = (rng.random::<f64>() + rng.random::<f64>() + rng.random::<f64>()) / 3.0;
                (10.0 + 80.0 * m.powi(3)) as usize
            })
            .collect(),
        None => {
            let base = rng.random_range(10..=20) as i64;
            (0..count)
                .map(|_| (base + rng.random_range(-6..=6)).max(5) as usize)
                .collect()
        }
    };

    GrowthParams { sizes, gamma, chaotic }
}

/// Board radius sized so the areas have room to grow plus buffer gaps.
pub fn grid_radius(params: &GrowthParams) -> i32 {
    let total: usize = params.sizes.iter().sum();
    let needed = (total * 3) as i32;
    let mut r = 4;
    while 3 * r * r + 3 * r + 1 < needed {
        r += 1;
    }
    (r + 2).min(40)
}

/// The grown areas. `cells[i]` lists area i's cells in growth order;
/// `kinds[i]` is its architectural state, kept aligned by construction.
pub struct Areas {
    pub cells: Vec<Vec<Hex>>,
    kinds: Vec<AreaKind>,
    owner: CellMap<u32>,
}

impl Areas {
    pub fn count(&self) -> usize {
        self.cells.len()
    }

    pub fn owner_of(&self, h: Hex) -> Option<usize> {
        self.owner.get(h).map(|o| o as usize)
    }

    /// The architectural state of area `i`.
    pub fn kind(&self, i: usize) -> AreaKind {
        self.kinds[i]
    }

    /// All kinds, aligned with `cells`.
    pub fn kinds(&self) -> &[AreaKind] {
        &self.kinds
    }

    /// Re-classify area `i` (e.g. demoted to organic when reshaping fails).
    pub fn set_kind(&mut self, i: usize, kind: AreaKind) {
        self.kinds[i] = kind;
    }

    /// Swap out an area's entire cell set (ruins reshaping). The new cells
    /// must be free or already owned by this area.
    pub fn replace_area(&mut self, i: usize, new_cells: Vec<Hex>) {
        for c in &self.cells[i] {
            self.owner.remove(*c);
        }
        for &c in &new_cells {
            debug_assert!(self.owner.get(c).is_none_or(|o| o as usize == i));
            self.owner.insert(c, i as u32);
        }
        self.cells[i] = new_cells;
    }

    /// Free the given cells of `area` (used by corridor shrinking).
    pub fn remove_from_area(&mut self, area: usize, remove: &[Hex]) {
        for &c in remove {
            self.owner.remove(c);
        }
        self.cells[area].retain(|c| !remove.contains(c));
    }
}

/// Areas smaller than this are discarded as failed growths.
const MIN_AREA: usize = 4;

/// Grow the areas. `slot_kinds[i]` is the pre-assigned kind of the area seeded
/// from `params.sizes[i]`; growth is kind-aware only in that two dungeon areas
/// may fuse ([`AreaKind::may_fuse`]). Slots that fail to grow (`MIN_AREA`) are
/// dropped; the surviving areas carry their kinds (`Areas::kind`).
pub fn grow_areas<R: Rng>(
    grid: &HexGrid,
    rng: &mut R,
    params: &GrowthParams,
    slot_kinds: &[AreaKind],
) -> Areas {
    let mut owner: CellMap<u32> = CellMap::new(grid.radius);
    let mut areas: Vec<Vec<Hex>> = Vec::new();
    let mut kinds: Vec<AreaKind> = Vec::new();

    for (slot, &target) in params.sizes.iter().enumerate() {
        let idx = areas.len();
        let cur_kind = slot_kinds[slot];

        let seed = if areas.is_empty() {
            // First area: seed anywhere whose whole neighbourhood is free,
            // preferring the centre so the cave grows outward in every
            // direction rather than hugging one side of the board.
            let valid: Vec<Hex> = grid
                .cells()
                .iter()
                .copied()
                .filter(|&h| {
                    !owner.contains(h)
                        && h.neighbors().iter().all(|n| !owner.contains(*n))
                })
                .collect();
            if valid.is_empty() {
                continue;
            }
            let central: Vec<Hex> = valid
                .iter()
                .copied()
                .filter(|h| h.distance(Hex::ORIGIN) <= grid.radius / 3)
                .collect();
            let seeds = if central.is_empty() { &valid } else { &central };
            seeds[rng.random_range(0..seeds.len())]
        } else {
            // Later areas seed one gap-cell away from the existing cave so
            // every area has a doorway candidate to a neighbour. Candidates
            // are exactly the cells at distance 2 from an owned cell, so
            // expand a two-cell ring outward from the owned set instead of
            // testing the whole board against every owned cell; sorting
            // restores the board-scan order, keeping RNG picks identical.
            let mut ring: Vec<Hex> = areas
                .iter()
                .flatten()
                .flat_map(|o| o.neighbors())
                .flat_map(|n| n.neighbors())
                .collect();
            ring.sort_unstable();
            ring.dedup();
            ring.retain(|&h| {
                grid.contains(h)
                    && !owner.contains(h)
                    && h.neighbors()
                        .iter()
                        .all(|n| neighbor_ok(owner.get(*n), idx, cur_kind, &kinds))
            });
            // No spot within reach of the existing cave: skip rather than
            // create an unreachable satellite area.
            if ring.is_empty() {
                continue;
            }
            ring[rng.random_range(0..ring.len())]
        };

        let mut cells = vec![seed];
        owner.insert(seed, idx as u32);

        // Frontier of valid candidates, kept sorted (BTreeSet iterates in
        // Hex order = the old sort order, so RNG picks are stable). Validity
        // is monotone while this area grows: adding our own cells never
        // invalidates a candidate, and a cell blocked by a non-fusable area
        // stays blocked (the other areas are already grown and fixed). So the
        // set only gains neighbours of newly added cells and loses the picked
        // cell; no per-step rebuild needed.
        let mut frontier: std::collections::BTreeSet<Hex> = std::collections::BTreeSet::new();
        extend_frontier(&mut frontier, grid, &owner, idx, cur_kind, &kinds, seed);

        while cells.len() < target {
            if frontier.is_empty() {
                break;
            }
            let cand: Vec<Hex> = frontier.iter().copied().collect();

            let weights: Vec<f64> = cand
                .iter()
                .map(|&h| {
                    let c = h
                        .neighbors()
                        .iter()
                        .filter(|n| owner.get(**n) == Some(idx as u32))
                        .count();
                    if params.chaotic {
                        if c == 2 { 8.0 } else { 1.0 }
                    } else {
                        (c as f64).powf(params.gamma)
                    }
                })
                .collect();

            let pick = cand[weighted_index(rng, &weights)];
            owner.insert(pick, idx as u32);
            cells.push(pick);
            frontier.remove(&pick);
            extend_frontier(&mut frontier, grid, &owner, idx, cur_kind, &kinds, pick);
        }

        if cells.len() < MIN_AREA {
            for c in &cells {
                owner.remove(*c);
            }
            continue;
        }
        areas.push(cells);
        kinds.push(cur_kind);
    }

    Areas { cells: areas, kinds, owner }
}

pub(crate) fn weighted_index<R: Rng>(rng: &mut R, weights: &[f64]) -> usize {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return rng.random_range(0..weights.len());
    }
    let mut t = rng.random_range(0.0..total);
    for (i, w) in weights.iter().enumerate() {
        t -= w;
        if t < 0.0 {
            return i;
        }
    }
    weights.len() - 1
}
