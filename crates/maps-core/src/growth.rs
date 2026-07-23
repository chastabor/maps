//! Staggered simultaneous growth. Areas are seeded a few at a time and all
//! grow together in rounds, so which seeds land first — and thus which areas
//! win the open space and grow large — varies from map to map. Dungeon rooms
//! grow as their true geometry (circles by rings, rectangles a side-strip at a
//! time); symmetric wings grow as **sibling orbits** in lockstep (a generator
//! plus its mirror/rotated copies, all adding the transformed increment each
//! round and stopping together), so a wing is always exactly symmetric and
//! self-sizing. Every area keeps a one-cell rock gap from every other, so the
//! gaps become doorways.

use crate::AreaKind;
use crate::grid::{CellMap, Hex, HexGrid};
use crate::ruins::RuinShape;
use crate::symmetry::{self, Xform};
use crate::tags::{LayoutTag, ShapeTag, SizeTag, Tags};
use rand::Rng;
use rand::seq::SliceRandom;
use std::collections::BTreeSet;

const SQRT3: f64 = crate::grid::SQRT3;
/// How far a **circular** dungeon room's drawn wall sits outside its outermost
/// cell centres, in hex-size units. (Rectangles align their walls to the hex
/// lattice instead — see `derive_shape`.)
const ROOM_WALL_PAD: f64 = 0.4;
/// Areas smaller than this are discarded as failed growths.
pub(crate) const MIN_AREA: usize = 4;
/// Organic areas add up to this many cells per round.
const ORGANIC_STEP: usize = 2;
// Dungeon rects are stamped down at a minimum 7-hex flower footprint up front
// (never grown into shape and found too thin), so a rect is born valid: a
// single-row rect would leave no interior once the wall is drawn inward and
// pinch its own neck into a bowtie. The flower spans 3 (odd) rows, so top and
// bottom walls share a stagger parity and stay aligned. See `rect_footprint`.

/// Growth-target multiplier. Seeds are spread across the board (see
/// `grow_areas`), so areas must grow larger than their nominal tag size to
/// reach neighbours and connect rather than be dropped as unreachable.
const GROWTH_BOOST: f64 = 1.5;

/// A candidate cell `c` may join area `idx` iff it is in-grid, free, and every
/// neighbour is free or already this area's — keeping the one-cell rock gap
/// from every other area.
fn placeable(grid: &HexGrid, owner: &CellMap<u32>, idx: u32, c: Hex) -> bool {
    grid.contains(c)
        && owner.get(c).is_none()
        && c.neighbors().iter().all(|n| owner.get(*n).is_none_or(|o| o == idx))
}

/// Whether area `idx` may claim **all** of `cells` this step, and — if the
/// claim drops a rock gap — which single partner it fuses with. Like
/// [`placeable`] but a **fusible same-kind** neighbour is allowed (the two fuse
/// into one compound, see [`AreaKind::may_fuse`]), subject to the **fuse-once**
/// rule: each area fuses with at most one partner. Returns
/// - `None` — blocked: a neighbour is a non-fusable/already-paired area, or the
///   batch would touch two different partners at once;
/// - `Some(None)` — claimable with the usual gap, no fusion;
/// - `Some(Some(o))` — claimable, fusing `idx` with partner `o` (the caller
///   records the pairing).
///
/// `meta[a] = (kind, fusible)` and `partner[a]` (the one area `a` has already
/// fused with, or `None`) are owned/separate slices, so the caller can hold
/// `&mut builds[idx]` while calling. Used only during expansion; seeding and
/// orbits keep `placeable`'s hard gap.
fn claim_batch(
    grid: &HexGrid,
    owner: &CellMap<u32>,
    meta: &[(AreaKind, bool)],
    partner: &[Option<u32>],
    idx: usize,
    cells: &[Hex],
) -> Option<Option<u32>> {
    // `idx`'s partner is fixed once set; a batch may only ever confirm it.
    let mut fuse_o = partner[idx];
    for &c in cells {
        if !grid.contains(c) || owner.get(c).is_some() {
            return None;
        }
        for n in c.neighbors() {
            let Some(o) = owner.get(n) else { continue };
            if o as usize == idx {
                continue;
            }
            let ou = o as usize;
            // The neighbour must be a fusible same-kind area that hasn't already
            // paired with someone other than `idx`.
            let eligible = meta[idx].1
                && meta[ou].1
                && meta[idx].0.may_fuse(meta[ou].0)
                && partner[ou].is_none_or(|p| p == idx as u32);
            if !eligible {
                return None;
            }
            match fuse_o {
                None => fuse_o = Some(o),
                Some(f) if f == o => {}
                Some(_) => return None, // would fuse `idx` with a second partner
            }
        }
    }
    Some(fuse_o)
}

// ---------------------------------------------------------------------------
// Dungeon room geometry — grown one increment (ring / side-strip) per round.
// ---------------------------------------------------------------------------

/// A dungeon room's growth state.
#[derive(Clone)]
enum Shape {
    /// Circle expanding by concentric rings; `r` is the current radius.
    Disk { c: (f64, f64), r: f64 },
    /// Rectangle expanding two rows / two columns at a time; extents are
    /// cell-centre bounds.
    Rect { c: (f64, f64), x0: f64, x1: f64, y0: f64, y1: f64 },
}

impl Shape {
    /// The candidate next-increments (geometric cells + the resulting state),
    /// each tagged `is_horizontal` for axis-first selection by the caller:
    /// one ring for a disk; for a rect, five moves that each add **two** rows
    /// or **two** columns.
    ///
    /// Every move keeps the rect an ODD number of rows/columns off its 3-row
    /// flower start, the parity that lets the wall corners land on hex vertices
    /// (see `derive_shape`). Vertical moves grow **two rows to one side** (up
    /// or down): splitting one row each side would push the outermost rows onto
    /// the *wide* stagger parity, and their vertices would no longer reach the
    /// corner x — the rect would stop lining up. Horizontal is parity-insensitive
    /// (every row shifts together), so besides two-left / two-right it may also
    /// grow one column on each side. The caller picks an axis 50/50 then a
    /// placement uniformly within it (¼ up, ¼ down, ⅙ each of left/right/split),
    /// so width and height grow in equal increments and a room stays square.
    fn candidates(&self, grid: &HexGrid, s: f64) -> Vec<(bool, Vec<Hex>, Shape)> {
        let col = SQRT3 * s;
        let row = 1.5 * s;
        let eps = 0.01 * s;
        match *self {
            Shape::Disk { c, r } => {
                let r2 = r + col;
                let ring: Vec<Hex> = grid
                    .cells()
                    .iter()
                    .copied()
                    .filter(|&h| {
                        let p = h.center(s);
                        let d = (p.0 - c.0).hypot(p.1 - c.1);
                        d > r + eps && d <= r2 + eps
                    })
                    .collect();
                vec![(false, ring, Shape::Disk { c, r: r2 })]
            }
            Shape::Rect { c, x0, x1, y0, y1 } => {
                // A move's cells are the grid cells whose centre falls in the
                // strip's world-rect. But a strip must be **complete** — fill
                // every lattice site the rectangle covers. Near the curved map
                // edge some sites lie off-grid; claiming just the in-grid ones
                // would leave a ragged room under a full bounding-box wall (a
                // triangular void). So we compare against the ideal lattice
                // count over the same world-rect and drop the move (empty
                // cells) unless it fills completely; the room then grows away
                // from the boundary and stays a filled rectangle.
                let mk = |is_h: bool,
                          pred: &dyn Fn((f64, f64)) -> bool,
                          bx0: f64, bx1: f64, by0: f64, by1: f64,
                          next: Shape|
                 -> (bool, Vec<Hex>, Shape) {
                    let strip: Vec<Hex> =
                        grid.cells().iter().copied().filter(|&h| pred(h.center(s))).collect();
                    let (r_lo, r_hi) =
                        ((by0 / row).floor() as i32 - 1, (by1 / row).ceil() as i32 + 1);
                    let mut ideal = 0usize;
                    for r in r_lo..=r_hi {
                        let q_lo = (bx0 / col - r as f64 / 2.0).floor() as i32 - 1;
                        let q_hi = (bx1 / col - r as f64 / 2.0).ceil() as i32 + 1;
                        for q in q_lo..=q_hi {
                            if pred(Hex { q, r }.center(s)) {
                                ideal += 1;
                            }
                        }
                    }
                    let cells = if strip.len() == ideal { strip } else { Vec::new() };
                    (is_h, cells, next)
                };
                let in_y = |y: f64| y >= y0 - eps && y <= y1 + eps;
                let in_x = |x: f64| x >= x0 - eps && x <= x1 + eps;
                let (h2, v2) = (2.0 * col, 2.0 * row);
                vec![
                    // Horizontal: two columns right / left / one on each side.
                    mk(
                        true,
                        &|p| p.0 > x1 + eps && p.0 <= x1 + h2 + eps && in_y(p.1),
                        x1, x1 + h2, y0, y1,
                        Shape::Rect { c, x0, x1: x1 + h2, y0, y1 },
                    ),
                    mk(
                        true,
                        &|p| p.0 < x0 - eps && p.0 >= x0 - h2 - eps && in_y(p.1),
                        x0 - h2, x0, y0, y1,
                        Shape::Rect { c, x0: x0 - h2, x1, y0, y1 },
                    ),
                    mk(
                        true,
                        &|p| {
                            in_y(p.1)
                                && ((p.0 > x1 + eps && p.0 <= x1 + col + eps)
                                    || (p.0 < x0 - eps && p.0 >= x0 - col - eps))
                        },
                        x0 - col, x1 + col, y0, y1,
                        Shape::Rect { c, x0: x0 - col, x1: x1 + col, y0, y1 },
                    ),
                    // Vertical: two rows up / down (never split — see above).
                    mk(
                        false,
                        &|p| p.1 > y1 + eps && p.1 <= y1 + v2 + eps && in_x(p.0),
                        x0, x1, y1, y1 + v2,
                        Shape::Rect { c, x0, x1, y0, y1: y1 + v2 },
                    ),
                    mk(
                        false,
                        &|p| p.1 < y0 - eps && p.1 >= y0 - v2 - eps && in_x(p.0),
                        x0, x1, y0 - v2, y0,
                        Shape::Rect { c, x0, x1, y0: y0 - v2, y1 },
                    ),
                ]
            }
        }
    }

    /// Order this shape's candidate moves for a growth attempt: pick an axis
    /// 50/50, then its placements uniformly, then the other axis as fallback
    /// (a disk's single ring is unaffected). The caller takes the first move
    /// whose cells it can actually claim.
    fn ordered_moves(
        &self,
        grid: &HexGrid,
        s: f64,
        rng: &mut impl Rng,
    ) -> Vec<(Vec<Hex>, Shape)> {
        let mut cands = self.candidates(grid, s);
        cands.shuffle(rng); // uniform placement within each axis
        let horiz_first = rng.random_bool(0.5);
        cands.sort_by_key(|&(is_h, ..)| is_h != horiz_first); // stable: chosen axis first
        cands.into_iter().map(|(_, cells, next)| (cells, next)).collect()
    }
}

/// The final wall shape from a room's cells: a rectangle's bounding box, or a
/// disk from the centroid and farthest cell. Deriving from cells makes a
/// sibling's wall the exact mirror of its generator's (means/bboxes commute
/// with the lattice transforms).
fn derive_shape(cells: &[Hex], is_rect: bool, s: f64) -> RuinShape {
    let pad = ROOM_WALL_PAD * s;
    if is_rect {
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for h in cells {
            let p = h.center(s);
            x0 = x0.min(p.0);
            x1 = x1.max(p.0);
            y0 = y0.min(p.1);
            y1 = y1.max(p.1);
        }
        // The wall rectangle is aligned to the hex lattice so its corners land
        // exactly on hex vertices and never overhang them. For pointy-top hexes
        // (vertices at 60k−30°): the left/right walls sit on the extreme cell
        // centres (no horizontal pad) — the corner then coincides with the
        // corner cell's 30°/150° vertex — and the top/bottom walls sit half a
        // hex above/below the extreme row centres (`s/2`, the side-vertex
        // offset), leaving the row peaks (at `s`) outside. The middle-row
        // extreme cell is bisected by the side wall; its overhang is clamped
        // back onto this rect by the outline splice (floor = the rect interior).
        RuinShape::Rect {
            cx: (x0 + x1) / 2.0,
            cy: (y0 + y1) / 2.0,
            hw: (x1 - x0) / 2.0,
            hh: (y1 - y0) / 2.0 + 0.5 * s,
        }
    } else {
        let (mut sx, mut sy) = (0.0, 0.0);
        for h in cells {
            let p = h.center(s);
            sx += p.0;
            sy += p.1;
        }
        let c = (sx / cells.len() as f64, sy / cells.len() as f64);
        let r = cells.iter().map(|h| { let p = h.center(s); (p.0 - c.0).hypot(p.1 - c.1) }).fold(0.0, f64::max);
        RuinShape::Circle { cx: c.0, cy: c.1, r: r + pad }
    }
}

/// The seven cells of a dungeon room's minimum footprint: `seed` plus its six
/// neighbours (the **flower** — the cells a circle occupies after one ring).
/// The room is stamped down at this size rather than grown into it, so it is
/// born valid: convex (no concave notch for the wall splice to preserve), 3
/// rows tall (odd, so top and bottom walls share a stagger parity), and every
/// rect corner lands inside an outer hex. Returns `None` if any cell is
/// blocked. The cells are not yet owned; the caller commits them.
fn flower_footprint(grid: &HexGrid, owner: &CellMap<u32>, idx: u32, seed: Hex) -> Option<Vec<Hex>> {
    let cells = flower_cells(seed);
    cells.iter().all(|&h| placeable(grid, owner, idx, h)).then_some(cells)
}

/// The seven cells of the flower footprint centred on `seed` (seed + its six
/// neighbours). The single definition of "minimum room = one-ring flower",
/// shared by the single-seed path and the symmetry-orbit path.
fn flower_cells(seed: Hex) -> Vec<Hex> {
    let mut cells = vec![seed];
    cells.extend(seed.neighbors());
    cells
}

/// The growth [`Shape`] a flower footprint starts from: a rect fitted to the
/// flower's bounding box, or a disk of one-ring radius. Both cover the same
/// seven cells; only the wall (and how it grows) differs.
fn footprint_shape(seed: Hex, s: f64, is_rect: bool) -> Shape {
    let c = seed.center(s);
    if is_rect {
        let hw = SQRT3 * s; // W..E span of the flower centres
        let hh = 1.5 * s; // N..S span
        Shape::Rect { c, x0: c.0 - hw, x1: c.0 + hw, y0: c.1 - hh, y1: c.1 + hh }
    } else {
        Shape::Disk { c, r: SQRT3 * s }
    }
}

// ---------------------------------------------------------------------------
// Growth parameters (resolved from tags) — unchanged.
// ---------------------------------------------------------------------------

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
            (0..count).map(|_| (base + rng.random_range(-2..=2)).max(5) as usize).collect()
        }
        Some(LayoutTag::Burrow) => (0..count)
            .map(|_| {
                let m = (rng.random::<f64>() + rng.random::<f64>() + rng.random::<f64>()) / 3.0;
                (10.0 + 80.0 * m.powi(3)) as usize
            })
            .collect(),
        None => {
            let base = rng.random_range(10..=20) as i64;
            (0..count).map(|_| (base + rng.random_range(-6..=6)).max(5) as usize).collect()
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

// ---------------------------------------------------------------------------
// The grown areas.
// ---------------------------------------------------------------------------

/// The grown areas. `cells[i]` lists area i's cells; `kinds[i]` is its
/// architectural state and `shapes[i]` its geometric wall (dungeon rooms get
/// theirs at growth, ruins at reshaping) — all kept aligned by construction.
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

    /// The geometric wall shape of area `i`, if it has one.
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

// ---------------------------------------------------------------------------
// The engine.
// ---------------------------------------------------------------------------

/// How one area advances each round.
enum Grow {
    Organic { frontier: BTreeSet<Hex> },
    /// An independent dungeon room.
    Shaped(Shape),
    /// A member of a sibling orbit — advanced by the orbit, not itself.
    Managed,
}

/// One growing area.
struct Build {
    cells: Vec<Hex>,
    kind: AreaKind,
    target: usize,
    active: bool,
    grow: Grow,
    /// For dungeon rooms: whether the wall is a rectangle (else a circle).
    is_rect: bool,
    /// Whether this area may grow into a same-kind fusible neighbour (no gap).
    fusible: bool,
}

/// A lockstep sibling orbit: the generator's shape drives all members, each
/// adding the transformed increment; if any member can't, all stop.
struct Orbit {
    centre: Hex,
    xforms: Vec<Xform>,
    members: Vec<usize>,
    shape: Shape,
    target: usize,
    active: bool,
}

/// One of four overlapping corner regions: the board's central third pushed
/// one sixth further toward a corner, covering about a quarter of the board.
/// Because each keeps the central third, all four overlap there (so seeds in
/// different drops can still connect); because each stops short of the far
/// edge, seeds never sit flush against the rim.
struct Section {
    /// Corner direction, each component ±1.
    sx: f64,
    sy: f64,
    /// The board's pixel half-extent in each axis.
    xmax: f64,
    ymax: f64,
}

impl Section {
    fn contains(&self, h: Hex, s: f64) -> bool {
        let (x, y) = h.center(s);
        let (dx, dy) = (x * self.sx, y * self.sy);
        dx >= -self.xmax / 3.0
            && dx <= 2.0 * self.xmax / 3.0
            && dy >= -self.ymax / 3.0
            && dy <= 2.0 * self.ymax / 3.0
    }
}

/// Grow the areas (see the module docs). `slot_kinds[i]` is the pre-assigned
/// kind of slot `i` and `slot_fusible[i]` whether it may fuse with a same-kind
/// neighbour it grows into; symmetry (chosen here from the shape stream) turns
/// some dungeon slots into sibling-orbit generators.
pub fn grow_areas<R: Rng>(
    grid: &HexGrid,
    rng: &mut R,
    params: &GrowthParams,
    slot_kinds: &[AreaKind],
    slot_fusible: &[bool],
    hex_size: f64,
) -> Areas {
    let mut owner: CellMap<u32> = CellMap::new(grid.radius);
    let mut builds: Vec<Build> = Vec::new();
    // `partner[a]` = the one area `a` has fused with (fuse-once). Grown in step
    // with `builds` each round.
    let mut partner: Vec<Option<u32>> = Vec::new();
    let mut orbits: Vec<Orbit> = Vec::new();

    // Symmetry plan + orbit centre (near the board centre, where wings fit).
    let plan = symmetry::choose(rng);
    let n_dungeon = slot_kinds.iter().filter(|&&k| k == AreaKind::Dungeon).count();
    let n_orbits = plan.as_ref().map_or(0, |p| p.generators.min(n_dungeon));
    let centre = {
        let cands: Vec<Hex> = grid.cells().iter().copied().filter(|&h| h.distance(Hex::ORIGIN) <= grid.radius / 3).collect();
        cands[rng.random_range(0..cands.len().max(1)).min(cands.len().saturating_sub(1))]
    };

    // Seeds are placed in three drops spread across four overlapping corner
    // sections (a quarter of the board each) rather than all crowding the
    // centre, so areas get room and fewer starve or fragment. Dungeon rooms
    // (and their symmetry orbits) are forced into the first drop for clean
    // space; the rest fill the three drops to roughly even thirds by count.
    // Each drop draws one section its single areas seed in — orbits ignore it
    // and radiate from `centre`, where symmetric wings fit.
    //
    // Spread seeds sit further apart, so targets are boosted 50%: bigger
    // areas close the gaps to their neighbours and connect (a doorway needs a
    // free cell touching both), instead of finishing short and being dropped
    // as an unreachable component. The board is not resized, so the extra
    // growth fills the room the sections opened up.
    // Each seeded single carries `(kind, target, fusible)`.
    let target = |i: usize| ((params.sizes[i] as f64 * GROWTH_BOOST).round() as usize).max(1);
    let mut dungeon_targets: Vec<(usize, bool)> = (0..params.sizes.len())
        .filter(|&i| slot_kinds[i] == AreaKind::Dungeon)
        .map(|i| (target(i), slot_fusible[i]))
        .collect();
    let mut orbit_pending: Vec<usize> = Vec::new();
    for _ in 0..n_orbits {
        // Orbit members never fuse (see `seed_orbit`), so their fusible flag is
        // dropped here.
        orbit_pending.push(dungeon_targets.pop().unwrap().0);
    }
    let n_dungeon_singles = dungeon_targets.len();
    let mut others: Vec<(AreaKind, usize, bool)> = (0..params.sizes.len())
        .filter(|&i| slot_kinds[i] != AreaKind::Dungeon)
        .map(|i| (slot_kinds[i], target(i), slot_fusible[i]))
        .collect();
    others.shuffle(rng);

    // Partition into three drops, ~even by total unit count (orbits counted
    // though seeded separately), dungeon singles forced into drop 0.
    let total = n_orbits + n_dungeon_singles + others.len();
    let third = total.div_ceil(3);
    let mut drops: [Vec<(AreaKind, usize, bool)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (t, fusible) in dungeon_targets {
        drops[0].push((AreaKind::Dungeon, t, fusible));
    }
    let mut d0 = n_orbits + drops[0].len();
    let mut rest = others.into_iter();
    while d0 < third {
        match rest.next() {
            Some(u) => {
                drops[0].push(u);
                d0 += 1;
            }
            None => break,
        }
    }
    let leftover: Vec<(AreaKind, usize, bool)> = rest.collect();
    let half = leftover.len().div_ceil(2);
    for (k, u) in leftover.into_iter().enumerate() {
        drops[if k < half { 1 } else { 2 }].push(u);
    }

    // One section per drop, drawn from the four overlapping corners.
    let corners = [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)];
    let (mut xmax, mut ymax) = (0.0_f64, 0.0_f64);
    for &h in grid.cells() {
        let (x, y) = h.center(hex_size);
        xmax = xmax.max(x.abs());
        ymax = ymax.max(y.abs());
    }
    let sections: Vec<Section> = (0..drops.len())
        .map(|_| {
            let (sx, sy) = corners[rng.random_range(0..corners.len())];
            Section { sx, sy, xmax, ymax }
        })
        .collect();

    let mut next_drop = 0usize;
    let mut seed_gap = 0u32;
    let mut rounds = 0u32;
    let max_rounds = 8 * params.sizes.len() as u32 + 400;
    loop {
        rounds += 1;
        // Seed the next drop once its gap elapses.
        if next_drop < drops.len() && seed_gap == 0 {
            // Orbits seed (and retry) at each drop moment, before that drop's
            // singles fill space — they need the cleanest room.
            if let Some(plan) = plan.as_ref() {
                orbit_pending.retain(|&target| {
                    !seed_orbit(grid, &mut owner, &mut builds, &mut orbits, plan, centre, target, hex_size, rng)
                });
            }
            let section = &sections[next_drop];
            for &(kind, target, fusible) in &drops[next_drop] {
                seed_single(grid, &mut owner, &mut builds, kind, target, fusible, section, hex_size, rng);
            }
            next_drop += 1;
            seed_gap = rng.random_range(1..=3);
        } else {
            seed_gap = seed_gap.saturating_sub(1);
        }

        // Advance every active independent area, then every active orbit.
        // `partner[a]` tracks the one area `a` has fused with (fuse-once), and
        // `meta` is a (kind, fusible) read-model — both per-area projections
        // owned by the round loop (kind/fusible never change after seeding), so
        // `advance_single` can consult all areas while holding `&mut builds[i]`.
        partner.resize(builds.len(), None);
        let meta: Vec<(AreaKind, bool)> = builds.iter().map(|b| (b.kind, b.fusible)).collect();
        let mut any_active = false;
        for i in 0..builds.len() {
            if builds[i].active {
                advance_single(grid, &mut owner, &mut builds, &mut partner, &meta, i, params, hex_size, rng);
                any_active |= builds[i].active;
            }
        }
        for o in 0..orbits.len() {
            if orbits[o].active {
                advance_orbit(grid, &mut owner, &mut builds, &mut orbits, o, hex_size, rng);
                any_active |= orbits[o].active;
            }
        }

        if next_drop >= drops.len() && !any_active {
            break;
        }
        if rounds >= max_rounds {
            break;
        }
    }

    let areas = finalize(grid, builds, hex_size);
    // Staggered growth can occasionally leave an area whose only gap-neighbour
    // was an under-sized area that got dropped, orphaning it. Keep only the
    // largest connected component so the door graph always spans the map.
    keep_largest_component(grid, areas)
}

/// Union-find root lookup with path halving, over a parent-index slice.
/// Union by re-rooting: `root[find(a)] = find(b)`.
pub(crate) fn find(root: &mut [usize], mut i: usize) -> usize {
    while root[i] != i {
        root[i] = root[root[i]];
        i = root[i];
    }
    i
}

/// Two areas are connected if a free cell touches both. Keep only the areas in
/// the largest such connected component (by cell count).
fn keep_largest_component(grid: &HexGrid, areas: Areas) -> Areas {
    let n = areas.count();
    if n <= 1 {
        return areas;
    }
    let mut root: Vec<usize> = (0..n).collect();
    for &h in grid.cells() {
        if areas.owner_of(h).is_some() {
            continue;
        }
        let mut adj: Vec<usize> = h.neighbors().iter().filter_map(|n| areas.owner_of(*n)).collect();
        adj.sort_unstable();
        adj.dedup();
        for w in adj.windows(2) {
            let (a, b) = (find(&mut root, w[0]), find(&mut root, w[1]));
            root[a] = b;
        }
    }
    // Component cell totals.
    let mut comp_cells: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for i in 0..n {
        let r = find(&mut root, i);
        *comp_cells.entry(r).or_default() += areas.cells[i].len();
    }
    let Some((&best, _)) = comp_cells.iter().max_by_key(|&(_, &v)| v) else {
        return areas;
    };
    let keep: Vec<bool> = (0..n).map(|i| find(&mut root, i) == best).collect();
    if keep.iter().all(|&k| k) {
        return areas;
    }
    // Rebuild with only the kept areas.
    let mut cells = Vec::new();
    let mut kinds = Vec::new();
    let mut shapes = Vec::new();
    let mut owner: CellMap<u32> = CellMap::new(grid.radius);
    for (i, keep_i) in keep.iter().enumerate() {
        if !keep_i {
            continue;
        }
        let idx = cells.len() as u32;
        for &c in &areas.cells[i] {
            owner.insert(c, idx);
        }
        cells.push(areas.cells[i].clone());
        kinds.push(areas.kinds[i]);
        shapes.push(areas.shapes[i]);
    }
    Areas { cells, kinds, shapes, owner }
}

/// Seed one independent area within `section` (or anywhere clean if the
/// section is full). Silently skips if no clean spot is free at all.
#[allow(clippy::too_many_arguments)]
fn seed_single<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    builds: &mut Vec<Build>,
    kind: AreaKind,
    target: usize,
    fusible: bool,
    section: &Section,
    hex_size: f64,
    rng: &mut R,
) {
    let idx = builds.len() as u32;
    // Dungeon AND ruin rooms are stamped down at their minimum flower footprint
    // so they are born valid — never grown into shape and found too thin (a
    // single-row rect leaves no interior and pinches its own neck) — and grow
    // as their exact geometry, hex-aligned to the lattice. Retry a few seed
    // spots. Organics seed at a point as before.
    if matches!(kind, AreaKind::Dungeon | AreaKind::Ruin) {
        let is_rect = rng.random_bool(0.5);
        for _ in 0..8 {
            let Some(seed) = pick_seed(grid, owner, idx, section, hex_size, rng) else { break };
            if let Some(cells) = flower_footprint(grid, owner, idx, seed) {
                for &c in &cells {
                    owner.insert(c, idx);
                }
                let shape = footprint_shape(seed, hex_size, is_rect);
                builds.push(Build { cells, kind, target, active: true, grow: Grow::Shaped(shape), is_rect, fusible });
                return;
            }
        }
        // No clean flower spot. A dungeon is skipped (it needs its clean shape);
        // a ruin falls back to an organic blob, demoted to Organic — a ruin with
        // no room for a footprint just weathers like the cave around it.
        if kind == AreaKind::Dungeon {
            return;
        }
    }
    let Some(seed) = pick_seed(grid, owner, idx, section, hex_size, rng) else { return };
    owner.insert(seed, idx);
    // A ruin reaching here couldn't take a shaped footprint: seed it organic.
    // Organic areas never fuse, so drop the fusible flag along with the kind.
    let (kind, fusible) = if kind == AreaKind::Ruin { (AreaKind::Organic, false) } else { (kind, fusible) };
    builds.push(Build {
        cells: vec![seed],
        kind,
        target,
        active: true,
        grow: Grow::Organic { frontier: BTreeSet::new() },
        is_rect: false,
        fusible,
    });
}

/// A seed cell for a new independent area: a random placeable cell inside
/// `section`, or — if the section has filled up — any placeable cell on the
/// board, so a crowded section costs distribution but never a whole area.
fn pick_seed<R: Rng>(
    grid: &HexGrid,
    owner: &CellMap<u32>,
    idx: u32,
    section: &Section,
    hex_size: f64,
    rng: &mut R,
) -> Option<Hex> {
    let in_section: Vec<Hex> = grid
        .cells()
        .iter()
        .copied()
        .filter(|&h| placeable(grid, owner, idx, h) && section.contains(h, hex_size))
        .collect();
    if !in_section.is_empty() {
        return Some(in_section[rng.random_range(0..in_section.len())]);
    }
    let any: Vec<Hex> = grid
        .cells()
        .iter()
        .copied()
        .filter(|&h| placeable(grid, owner, idx, h))
        .collect();
    (!any.is_empty()).then(|| any[rng.random_range(0..any.len())])
}

/// Advance one independent area by a single round's increment.
#[allow(clippy::too_many_arguments)]
fn advance_single<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    builds: &mut [Build],
    partner: &mut [Option<u32>],
    // Per-round (kind, fusible) snapshot: an owned/separate read-model so a
    // fusible area may consult every area while `builds[i]` is borrowed mutably.
    meta: &[(AreaKind, bool)],
    i: usize,
    params: &GrowthParams,
    hex_size: f64,
    rng: &mut R,
) {
    if builds[i].cells.len() >= builds[i].target {
        builds[i].active = false;
        return;
    }
    // Record a fusion between `idx` and `o` (both ends), so neither pairs again.
    let pair = |partner: &mut [Option<u32>], idx: usize, o: u32| {
        partner[idx] = Some(o);
        partner[o as usize] = Some(idx as u32);
    };
    match &mut builds[i].grow {
        Grow::Managed => {}
        Grow::Organic { .. } => {
            let mut added = 0;
            while added < ORGANIC_STEP {
                // Rebuild frontier candidates lazily: re-check placeability
                // (neighbours may have been claimed by a concurrent area).
                let frontier: Vec<Hex> = match &builds[i].grow {
                    Grow::Organic { frontier } => frontier.iter().copied().collect(),
                    _ => break,
                };
                // Each candidate carries the partner it would fuse with, so the
                // chosen cell needn't re-run the eligibility scan.
                let cand: Vec<(Hex, Option<u32>)> = frontier
                    .into_iter()
                    .filter_map(|h| claim_batch(grid, owner, meta, partner, i, &[h]).map(|ft| (h, ft)))
                    .collect();
                if cand.is_empty() {
                    // Seed the frontier from the current boundary once, else stop.
                    if builds[i].cells.len() == 1 {
                        let seed = builds[i].cells[0];
                        if let Grow::Organic { frontier } = &mut builds[i].grow {
                            for n in seed.neighbors() {
                                if claim_batch(grid, owner, meta, partner, i, &[n]).is_some() {
                                    frontier.insert(n);
                                }
                            }
                        }
                        if matches!(&builds[i].grow, Grow::Organic { frontier } if frontier.is_empty()) {
                            builds[i].active = false;
                            return;
                        }
                        continue;
                    }
                    builds[i].active = false;
                    return;
                }
                let weights: Vec<f64> = cand
                    .iter()
                    .map(|&(h, _)| {
                        let c = h.neighbors().iter().filter(|n| owner.get(**n) == Some(i as u32)).count();
                        if params.chaotic {
                            if c == 2 { 8.0 } else { 1.0 }
                        } else {
                            (c as f64).powf(params.gamma)
                        }
                    })
                    .collect();
                let (pick, ft) = cand[weighted_index(rng, &weights)];
                owner.insert(pick, i as u32);
                builds[i].cells.push(pick);
                if let Some(o) = ft {
                    pair(partner, i, o);
                }
                if let Grow::Organic { frontier } = &mut builds[i].grow {
                    frontier.remove(&pick);
                    for n in pick.neighbors() {
                        if claim_batch(grid, owner, meta, partner, i, &[n]).is_some() {
                            frontier.insert(n);
                        }
                    }
                }
                added += 1;
            }
        }
        Grow::Shaped(shape) => {
            let cands = shape.ordered_moves(grid, hex_size, rng);
            let mut grew = false;
            for (cells, next) in cands {
                if cells.is_empty() {
                    continue;
                }
                if let Some(ft) = claim_batch(grid, owner, meta, partner, i, &cells) {
                    for &c in &cells {
                        owner.insert(c, i as u32);
                    }
                    builds[i].cells.extend(cells);
                    builds[i].grow = Grow::Shaped(next);
                    if let Some(o) = ft {
                        pair(partner, i, o);
                    }
                    grew = true;
                    break;
                }
            }
            if !grew {
                builds[i].active = false;
            }
        }
    }
}

/// Advance one sibling orbit in lockstep: the generator's next shape
/// increment, mirrored to every member; committed only if all members' cells
/// are placeable and no two members' new cells touch, else the orbit stops.
#[allow(clippy::too_many_arguments)]
fn advance_orbit<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    builds: &mut [Build],
    orbits: &mut [Orbit],
    o: usize,
    hex_size: f64,
    rng: &mut R,
) {
    let gen_i = orbits[o].members[0];
    if builds[gen_i].cells.len() >= orbits[o].target {
        orbits[o].active = false;
        return;
    }
    let cands = orbits[o].shape.ordered_moves(grid, hex_size, rng);
    for (gen_cells, next) in cands {
        if gen_cells.is_empty() {
            continue;
        }
        // Each member's new cells = the generator's, transformed.
        let mut per_member: Vec<Vec<Hex>> = Vec::with_capacity(orbits[o].members.len());
        for &xf in &orbits[o].xforms {
            per_member.push(gen_cells.iter().map(|&c| xf.cell(orbits[o].centre, c)).collect());
        }
        // Validate: every cell placeable for its member, and no two new cells
        // from different members coincide or touch (keeps the sibling gap).
        let mut all: Vec<(usize, Hex)> = Vec::new();
        let mut ok = true;
        'v: for (m, cells) in per_member.iter().enumerate() {
            let midx = orbits[o].members[m] as u32;
            for &c in cells {
                if !placeable(grid, owner, midx, c) {
                    ok = false;
                    break 'v;
                }
                all.push((m, c));
            }
        }
        if ok {
            let set: std::collections::HashSet<Hex> = all.iter().map(|&(_, c)| c).collect();
            if set.len() != all.len() {
                ok = false; // two members want the same cell
            } else {
                for &(m, c) in &all {
                    for n in c.neighbors() {
                        if all.iter().any(|&(m2, c2)| m2 != m && c2 == n) {
                            ok = false;
                            break;
                        }
                    }
                    if !ok {
                        break;
                    }
                }
            }
        }
        if ok {
            for (m, cells) in per_member.into_iter().enumerate() {
                let midx = orbits[o].members[m];
                for &c in &cells {
                    owner.insert(c, midx as u32);
                }
                builds[midx].cells.extend(cells);
            }
            orbits[o].shape = next;
            return;
        }
    }
    // No increment worked for the whole orbit — everyone stops together.
    orbits[o].active = false;
    for &m in &orbits[o].members {
        builds[m].active = false;
    }
}

/// Try to seed a sibling orbit: place the generator at a random radius from
/// `centre` and its siblings at the transformed positions, all on clean free
/// cells. Returns whether it placed.
#[allow(clippy::too_many_arguments)]
fn seed_orbit<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    builds: &mut Vec<Build>,
    orbits: &mut Vec<Orbit>,
    plan: &symmetry::SymPlan,
    centre: Hex,
    target: usize,
    hex_size: f64,
    rng: &mut R,
) -> bool {
    // Radial orbits (rotations that don't keep a rect) must be disks.
    let allow_rect = plan.xforms.iter().all(|x| x.keeps_rect());
    let ring: Vec<Hex> = grid
        .cells()
        .iter()
        .copied()
        .filter(|&h| {
            let d = h.distance(centre);
            (3..=8).contains(&d)
        })
        .collect();
    for _ in 0..40 {
        if ring.is_empty() {
            return false;
        }
        let gen_seed = ring[rng.random_range(0..ring.len())];
        // Every member is stamped down at the full flower footprint up front —
        // mirrored from the generator's — and the orbit only seeds if the
        // *whole* symmetric set fits (all cells placeable, and members' cells
        // keep a one-cell gap). Checking the footprints together means a wing
        // never seeds only to fail growing into shape later.
        let gen_flower = flower_cells(gen_seed);
        let per_member: Vec<Vec<Hex>> = plan
            .xforms
            .iter()
            .map(|xf| gen_flower.iter().map(|&c| xf.cell(centre, c)).collect())
            .collect();
        let base = builds.len() as u32;
        let all: Vec<Hex> = per_member.iter().flatten().copied().collect();
        let uniq: std::collections::HashSet<Hex> = all.iter().copied().collect();
        if uniq.len() != all.len() {
            continue;
        }
        let placeable_ok = per_member
            .iter()
            .enumerate()
            .all(|(k, cells)| cells.iter().all(|&c| placeable(grid, owner, base + k as u32, c)));
        if !placeable_ok {
            continue;
        }
        // Members' cells keep a one-cell gap from each other.
        let mut gap_ok = true;
        'pair: for a in 0..per_member.len() {
            for b in a + 1..per_member.len() {
                for &ca in &per_member[a] {
                    for &cb in &per_member[b] {
                        if ca.distance(cb) < 2 {
                            gap_ok = false;
                            break 'pair;
                        }
                    }
                }
            }
        }
        if !gap_ok {
            continue;
        }
        // Commit: generator (member 0) first, then siblings, all as flowers.
        let is_rect = allow_rect && rng.random_bool(0.5);
        let shape = footprint_shape(gen_seed, hex_size, is_rect);
        let mut members = Vec::new();
        for (k, cells) in per_member.into_iter().enumerate() {
            let idx = builds.len();
            for &c in &cells {
                owner.insert(c, idx as u32);
            }
            builds.push(Build {
                cells,
                kind: AreaKind::Dungeon,
                target,
                active: k == 0, // only the generator's `active` matters (orbit drives)
                grow: Grow::Managed,
                is_rect,
                // Orbit members keep the gap (fusion is for independent growth
                // only), so lockstep symmetric wings stay clean.
                fusible: false,
            });
            members.push(idx);
        }
        orbits.push(Orbit { centre, xforms: plan.xforms.clone(), members, shape, target, active: true });
        return true;
    }
    false
}

/// Drop under-sized areas, re-index, derive dungeon wall shapes, and build the
/// final `Areas`.
fn finalize(grid: &HexGrid, builds: Vec<Build>, hex_size: f64) -> Areas {
    let mut cells: Vec<Vec<Hex>> = Vec::new();
    let mut kinds: Vec<AreaKind> = Vec::new();
    let mut shapes: Vec<Option<RuinShape>> = Vec::new();
    let mut owner: CellMap<u32> = CellMap::new(grid.radius);
    for b in builds {
        if b.cells.len() < MIN_AREA {
            continue;
        }
        let idx = cells.len() as u32;
        for &c in &b.cells {
            owner.insert(c, idx);
        }
        // Dungeon and ruin rooms both grew from a flower into their exact
        // geometry, so both derive a hex-aligned wall shape the same way. (A
        // ruin that fell back to organic growth has is_rect=false but is kind
        // Organic, so it takes no shape here.)
        let shape = matches!(b.kind, AreaKind::Dungeon | AreaKind::Ruin)
            .then(|| derive_shape(&b.cells, b.is_rect, hex_size));
        cells.push(b.cells);
        kinds.push(b.kind);
        shapes.push(shape);
    }
    Areas { cells, kinds, shapes, owner }
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
