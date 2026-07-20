//! Seed-growth: grow N disjoint areas cell by cell, keeping a one-cell gap
//! between areas so that gap cells can later become doorways.

use crate::AreaKind;
use crate::grid::{CellMap, Hex, HexGrid};
use crate::ruins::RuinShape;
use crate::tags::{LayoutTag, ShapeTag, SizeTag, Tags};
use rand::Rng;
use rand::seq::SliceRandom;

/// Probability that a dungeon area is *open to fusing* with a neighbouring
/// dungeon room (rolled once per area). Two rooms actually fuse only when
/// **both** rolled open, so compound shapes (rect + silo, ...) appear at
/// roughly `FUSE_P²` of dungeon adjacencies and most rooms stay distinct.
const FUSE_P: f64 = 0.7;

/// May the growing area `idx` (of kind `cur_kind`, fusion allowance `fusing`)
/// sit next to the cell owned by `owner_val`? Free cells and its own cells
/// are fine; a foreign area only if the kinds fuse ([`AreaKind::may_fuse`] —
/// dungeon↔dungeon only) and **both** sides rolled open to fusion (`fusing`
/// for the grower, `fuse[o]` for the neighbour).
fn neighbor_ok(
    owner_val: Option<u32>,
    idx: usize,
    cur_kind: AreaKind,
    kinds: &[AreaKind],
    fuse: &[bool],
    fusing: bool,
) -> bool {
    owner_val.is_none_or(|o| {
        let o = o as usize;
        o == idx || (fusing && fuse[o] && cur_kind.may_fuse(kinds[o]))
    })
}

/// How far a dungeon room's drawn wall sits outside its outermost cell
/// centres, in hex-size units. The wall cuts through the boundary hexes —
/// the cells are the ownership raster, the wall is the true geometry.
const ROOM_WALL_PAD: f64 = 0.4;

/// Grow a dungeon room as a **true geometric shape**: a circle expanding by
/// concentric rings, or a rectangle expanding one whole side-strip at a time
/// (in a random side order). Every increment is all-or-nothing — if any cell
/// of the next ring/strip is out of grid, taken, or would violate the rock
/// gap toward a non-fusable area, that increment is refused (a rectangle
/// tries its other sides; a circle stops). The footprint therefore stays an
/// exact rect/circle raster at every step, and the room can never deform
/// around a neighbour. Returns the claimed cells plus the shape to draw
/// (in pixel space), whose walls the outline projects onto.
#[allow(clippy::too_many_arguments)]
fn grow_room<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    idx: usize,
    kinds: &[AreaKind],
    fuse: &[bool],
    fusing: bool,
    seed: Hex,
    target: usize,
    hex_size: f64,
    rng: &mut R,
) -> (Vec<Hex>, RuinShape) {
    let s = hex_size;
    let col = crate::grid::SQRT3 * s; // horizontal centre pitch
    let row = 1.5 * s; // vertical centre pitch
    let eps = 0.01 * s;
    let c = seed.center(s);
    let mut cells = vec![seed];
    owner.insert(seed, idx as u32);

    let admissible = |owner: &CellMap<u32>, h: Hex| {
        !owner.contains(h)
            && h.neighbors()
                .iter()
                .all(|n| neighbor_ok(owner.get(*n), idx, AreaKind::Dungeon, kinds, fuse, fusing))
    };

    if rng.random_bool(0.5) {
        // Circle: concentric rings. One lattice shell per step (all six
        // neighbours sit √3·s from a centre).
        let mut r = 0.9 * s;
        while cells.len() < target {
            let r2 = r + col + eps;
            let ring: Vec<Hex> = grid
                .cells()
                .iter()
                .copied()
                .filter(|&h| {
                    !owner.contains(h) && {
                        let p = h.center(s);
                        (p.0 - c.0).hypot(p.1 - c.1) <= r2
                    }
                })
                .collect();
            if ring.is_empty() || !ring.iter().all(|&h| admissible(owner, h)) {
                break;
            }
            for &h in &ring {
                owner.insert(h, idx as u32);
                cells.push(h);
            }
            r = r2;
        }
        let r_max = cells
            .iter()
            .map(|h| {
                let p = h.center(s);
                (p.0 - c.0).hypot(p.1 - c.1)
            })
            .fold(0.0, f64::max);
        (cells, RuinShape::Circle { cx: c.0, cy: c.1, r: r_max + ROOM_WALL_PAD * s })
    } else {
        // Rectangle: expand one whole side-strip at a time. The box tracks
        // cell-centre extents; a strip is every free cell whose centre falls
        // in the next column/row band.
        let (mut x0, mut x1, mut y0, mut y1) = (c.0, c.0, c.1, c.1);
        let mut order: Vec<usize> = (0..4).collect();
        while cells.len() < target {
            order.shuffle(rng);
            let mut grew = false;
            for &side in &order {
                let band = |p: (f64, f64)| match side {
                    0 => p.0 > x1 + eps && p.0 <= x1 + col + eps && p.1 >= y0 - eps && p.1 <= y1 + eps,
                    1 => p.0 < x0 - eps && p.0 >= x0 - col - eps && p.1 >= y0 - eps && p.1 <= y1 + eps,
                    2 => p.1 > y1 + eps && p.1 <= y1 + row + eps && p.0 >= x0 - eps && p.0 <= x1 + eps,
                    _ => p.1 < y0 - eps && p.1 >= y0 - row - eps && p.0 >= x0 - eps && p.0 <= x1 + eps,
                };
                let strip: Vec<Hex> =
                    grid.cells().iter().copied().filter(|&h| band(h.center(s))).collect();
                if strip.is_empty() || !strip.iter().all(|&h| admissible(owner, h)) {
                    continue;
                }
                for &h in &strip {
                    owner.insert(h, idx as u32);
                    cells.push(h);
                }
                match side {
                    0 => x1 += col,
                    1 => x0 -= col,
                    2 => y1 += row,
                    _ => y0 -= row,
                }
                grew = true;
                break;
            }
            if !grew {
                break;
            }
        }
        // The staggered rows put centres up to half a column pitch outside
        // the tracked extents; measure the real extremes for the wall.
        let (mut ex0, mut ex1, mut ey0, mut ey1) = (c.0, c.0, c.1, c.1);
        for h in &cells {
            let p = h.center(s);
            ex0 = ex0.min(p.0);
            ex1 = ex1.max(p.0);
            ey0 = ey0.min(p.1);
            ey1 = ey1.max(p.1);
        }
        let pad = ROOM_WALL_PAD * s;
        (
            cells,
            RuinShape::Rect {
                cx: (ex0 + ex1) / 2.0,
                cy: (ey0 + ey1) / 2.0,
                hw: (ex1 - ex0) / 2.0 + pad,
                hh: (ey1 - ey0) / 2.0 + pad,
            },
        )
    }
}

/// Extend `frontier` with the still-valid free neighbours of newly added cell
/// `c`: in-grid, unowned, and every one of their own neighbours passes
/// [`neighbor_ok`].
#[allow(clippy::too_many_arguments)]
fn extend_frontier(
    frontier: &mut std::collections::BTreeSet<Hex>,
    grid: &HexGrid,
    owner: &CellMap<u32>,
    idx: usize,
    cur_kind: AreaKind,
    kinds: &[AreaKind],
    fuse: &[bool],
    fusing: bool,
    c: Hex,
) {
    for n in c.neighbors() {
        if grid.contains(n)
            && !owner.contains(n)
            && n.neighbors()
                .iter()
                .all(|m| neighbor_ok(owner.get(*m), idx, cur_kind, kinds, fuse, fusing))
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
/// `kinds[i]` is its architectural state and `shapes[i]` its geometric wall
/// (dungeon rooms get theirs at growth, ruins at reshaping) — all kept
/// aligned by construction.
pub struct Areas {
    pub cells: Vec<Vec<Hex>>,
    kinds: Vec<AreaKind>,
    shapes: Vec<Option<RuinShape>>,
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

    /// The geometric wall shape of area `i`, if it has one (dungeon rooms
    /// always; ruins once reshaped; organic never).
    pub fn shape(&self, i: usize) -> Option<RuinShape> {
        self.shapes[i]
    }

    /// All shapes, aligned with `cells`.
    pub fn shapes(&self) -> &[Option<RuinShape>] {
        &self.shapes
    }

    /// Record area `i`'s geometric wall (ruin reshaping).
    pub fn set_shape(&mut self, i: usize, shape: Option<RuinShape>) {
        self.shapes[i] = shape;
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
/// from `params.sizes[i]`; growth is kind-aware in two ways: two dungeon areas
/// may fuse ([`AreaKind::may_fuse`]), and dungeon areas grow *as their true
/// room geometry* ([`grow_room`] — circles by concentric rings, rectangles a
/// side-strip at a time), seeded before everything else so the rooms have
/// space to come out clean. Slots that fail to grow (`MIN_AREA`) are dropped;
/// the surviving areas carry their kinds and shapes (`Areas::kind`/`shape`).
pub fn grow_areas<R: Rng>(
    grid: &HexGrid,
    rng: &mut R,
    params: &GrowthParams,
    slot_kinds: &[AreaKind],
    hex_size: f64,
) -> Areas {
    let mut owner: CellMap<u32> = CellMap::new(grid.radius);
    let mut areas: Vec<Vec<Hex>> = Vec::new();
    let mut kinds: Vec<AreaKind> = Vec::new();
    let mut shapes: Vec<Option<RuinShape>> = Vec::new();
    let mut fuse: Vec<bool> = Vec::new();

    // Dungeon rooms first (they need clean space), then the rest, each group
    // in slot order.
    let order: Vec<usize> = (0..params.sizes.len())
        .filter(|&i| slot_kinds[i] == AreaKind::Dungeon)
        .chain((0..params.sizes.len()).filter(|&i| slot_kinds[i] != AreaKind::Dungeon))
        .collect();
    for &slot in &order {
        let target = params.sizes[slot];
        let idx = areas.len();
        let cur_kind = slot_kinds[slot];
        // Dungeon rooms roll whether they are open to fusing with a
        // neighbouring room; a pair fuses only if both rolled open.
        let fusing = cur_kind == AreaKind::Dungeon && rng.random_bool(FUSE_P);

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
                        .all(|n| neighbor_ok(owner.get(*n), idx, cur_kind, &kinds, &fuse, fusing))
            });
            // No spot within reach of the existing cave: skip rather than
            // create an unreachable satellite area.
            if ring.is_empty() {
                continue;
            }
            ring[rng.random_range(0..ring.len())]
        };

        // Dungeon rooms grow as their true geometry from the seed; the rest
        // grow organically over a frontier.
        let (cells, shape) = if cur_kind == AreaKind::Dungeon {
            let (cells, shape) =
                grow_room(grid, &mut owner, idx, &kinds, &fuse, fusing, seed, target, hex_size, rng);
            (cells, Some(shape))
        } else {
            let mut cells = vec![seed];
            owner.insert(seed, idx as u32);

            // Frontier of valid candidates, kept sorted (BTreeSet iterates in
            // Hex order = the old sort order, so RNG picks are stable).
            // Validity is monotone while this area grows: adding our own
            // cells never invalidates a candidate, and a cell blocked by a
            // non-fusable area stays blocked (the other areas are already
            // grown and fixed). So the set only gains neighbours of newly
            // added cells and loses the picked cell; no per-step rebuild.
            let mut frontier: std::collections::BTreeSet<Hex> = std::collections::BTreeSet::new();
            extend_frontier(&mut frontier, grid, &owner, idx, cur_kind, &kinds, &fuse, fusing, seed);

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
                extend_frontier(&mut frontier, grid, &owner, idx, cur_kind, &kinds, &fuse, fusing, pick);
            }
            (cells, None)
        };

        if cells.len() < MIN_AREA {
            for c in &cells {
                owner.remove(*c);
            }
            continue;
        }
        // A fusion allowance is spent when it is used: if this room actually
        // grew onto another dungeon room, close both sides to further fusion
        // so compounds stay pairwise (a rect with its silo) instead of
        // percolating the whole dungeon into one blob.
        let mut spent = false;
        if fusing {
            let partners: Vec<usize> = cells
                .iter()
                .flat_map(|c| c.neighbors())
                .filter_map(|n| owner.get(n).map(|o| o as usize))
                .filter(|&o| o != idx)
                .collect();
            for o in partners {
                fuse[o] = false;
                spent = true;
            }
        }
        areas.push(cells);
        kinds.push(cur_kind);
        shapes.push(shape);
        fuse.push(fusing && !spent);
    }

    Areas { cells: areas, kinds, shapes, owner }
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
