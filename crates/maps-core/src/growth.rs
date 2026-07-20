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
/// How far a dungeon room's drawn wall sits outside its outermost cell
/// centres, in hex-size units — the cells are the ownership raster, the wall
/// is the true geometry cutting through the boundary hexes.
const ROOM_WALL_PAD: f64 = 0.4;
/// Areas smaller than this are discarded as failed growths.
const MIN_AREA: usize = 4;
/// Organic areas add up to this many cells per round.
const ORGANIC_STEP: usize = 2;

/// A candidate cell `c` may join area `idx` iff it is in-grid, free, and every
/// neighbour is free or already this area's — keeping the one-cell rock gap
/// from every other area.
fn placeable(grid: &HexGrid, owner: &CellMap<u32>, idx: u32, c: Hex) -> bool {
    grid.contains(c)
        && owner.get(c).is_none()
        && c.neighbors().iter().all(|n| owner.get(*n).is_none_or(|o| o == idx))
}

// ---------------------------------------------------------------------------
// Dungeon room geometry — grown one increment (ring / side-strip) per round.
// ---------------------------------------------------------------------------

/// A dungeon room's growth state.
#[derive(Clone)]
enum Shape {
    /// Circle expanding by concentric rings; `r` is the current radius.
    Disk { c: (f64, f64), r: f64 },
    /// Rectangle expanding one side-strip at a time; extents are cell-centre
    /// bounds. `order` is the side try-order (reshuffled each round).
    Rect { c: (f64, f64), x0: f64, x1: f64, y0: f64, y1: f64, order: [usize; 4] },
}

impl Shape {
    fn is_rect(&self) -> bool {
        matches!(self, Shape::Rect { .. })
    }

    /// The candidate next-increments (geometric cells + the resulting state):
    /// one ring for a disk, one per untried side for a rect (in `order`).
    fn candidates(&self, grid: &HexGrid, s: f64) -> Vec<(Vec<Hex>, Shape)> {
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
                vec![(ring, Shape::Disk { c, r: r2 })]
            }
            Shape::Rect { c, x0, x1, y0, y1, order } => order
                .iter()
                .map(|&side| {
                    let band = |p: (f64, f64)| match side {
                        0 => p.0 > x1 + eps && p.0 <= x1 + col + eps && p.1 >= y0 - eps && p.1 <= y1 + eps,
                        1 => p.0 < x0 - eps && p.0 >= x0 - col - eps && p.1 >= y0 - eps && p.1 <= y1 + eps,
                        2 => p.1 > y1 + eps && p.1 <= y1 + row + eps && p.0 >= x0 - eps && p.0 <= x1 + eps,
                        _ => p.1 < y0 - eps && p.1 >= y0 - row - eps && p.0 >= x0 - eps && p.0 <= x1 + eps,
                    };
                    let strip: Vec<Hex> = grid.cells().iter().copied().filter(|&h| band(h.center(s))).collect();
                    let mut next = Shape::Rect { c, x0, x1, y0, y1, order };
                    if let Shape::Rect { x0, x1, y0, y1, .. } = &mut next {
                        match side {
                            0 => *x1 += col,
                            1 => *x0 -= col,
                            2 => *y1 += row,
                            _ => *y0 -= row,
                        }
                    }
                    (strip, next)
                })
                .collect(),
        }
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
        RuinShape::Rect { cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, hw: (x1 - x0) / 2.0 + pad, hh: (y1 - y0) / 2.0 + pad }
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

/// A unit still waiting to be seeded.
enum Unit {
    Single { kind: AreaKind, target: usize },
    Orbit { target: usize },
}

/// Grow the areas (see the module docs). `slot_kinds[i]` is the pre-assigned
/// kind of slot `i`; symmetry (chosen here from the shape stream) turns some
/// dungeon slots into sibling-orbit generators.
pub fn grow_areas<R: Rng>(
    grid: &HexGrid,
    rng: &mut R,
    params: &GrowthParams,
    slot_kinds: &[AreaKind],
    hex_size: f64,
) -> Areas {
    let mut owner: CellMap<u32> = CellMap::new(grid.radius);
    let mut builds: Vec<Build> = Vec::new();
    let mut orbits: Vec<Orbit> = Vec::new();

    // Symmetry plan + orbit centre (near the board centre, where wings fit).
    let plan = symmetry::choose(rng);
    let n_dungeon = slot_kinds.iter().filter(|&&k| k == AreaKind::Dungeon).count();
    let n_orbits = plan.as_ref().map_or(0, |p| p.generators.min(n_dungeon));
    let centre = {
        let cands: Vec<Hex> = grid.cells().iter().copied().filter(|&h| h.distance(Hex::ORIGIN) <= grid.radius / 3).collect();
        cands[rng.random_range(0..cands.len().max(1)).min(cands.len().saturating_sub(1))]
    };

    // Build the seed queue: orbit generators first (they need clean space),
    // then the remaining areas shuffled so dungeon/ruin/organic interleave.
    let mut dungeon_targets: Vec<usize> =
        (0..params.sizes.len()).filter(|&i| slot_kinds[i] == AreaKind::Dungeon).map(|i| params.sizes[i]).collect();
    let mut queue: Vec<Unit> = Vec::new();
    for _ in 0..n_orbits {
        queue.push(Unit::Orbit { target: dungeon_targets.pop().unwrap() });
    }
    let mut rest: Vec<Unit> = Vec::new();
    for t in dungeon_targets {
        rest.push(Unit::Single { kind: AreaKind::Dungeon, target: t });
    }
    for (i, &k) in slot_kinds.iter().enumerate() {
        if k != AreaKind::Dungeon {
            rest.push(Unit::Single { kind: k, target: params.sizes[i] });
        }
    }
    rest.shuffle(rng);
    queue.extend(rest);
    queue.reverse(); // pop() takes from the front

    let mut seed_gap = 0u32;
    let mut rounds = 0u32;
    let max_rounds = 8 * params.sizes.len() as u32 + 400;
    loop {
        rounds += 1;
        // Seed a batch every 1–3 rounds.
        if !queue.is_empty() && seed_gap == 0 {
            let batch = rng.random_range(1..=3);
            for _ in 0..batch {
                let Some(unit) = queue.pop() else { break };
                match unit {
                    Unit::Single { kind, target } => {
                        seed_single(grid, &mut owner, &mut builds, kind, target, hex_size, rng);
                    }
                    Unit::Orbit { target } => {
                        if !seed_orbit(grid, &mut owner, &mut builds, &mut orbits, plan.as_ref().unwrap(), centre, target, hex_size, rng) {
                            // No room right now; try again in a later batch.
                            queue.insert(0, Unit::Orbit { target });
                        }
                    }
                }
            }
            seed_gap = rng.random_range(1..=3);
        } else {
            seed_gap = seed_gap.saturating_sub(1);
        }

        // Advance every active independent area, then every active orbit.
        let mut any_active = false;
        for i in 0..builds.len() {
            if builds[i].active {
                advance_single(grid, &mut owner, &mut builds, i, params, hex_size, rng);
                any_active |= builds[i].active;
            }
        }
        for o in 0..orbits.len() {
            if orbits[o].active {
                advance_orbit(grid, &mut owner, &mut builds, &mut orbits, o, hex_size, rng);
                any_active |= orbits[o].active;
            }
        }

        if queue.is_empty() && !any_active {
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

/// Two areas are connected if a free cell touches both. Keep only the areas in
/// the largest such connected component (by cell count).
fn keep_largest_component(grid: &HexGrid, areas: Areas) -> Areas {
    let n = areas.count();
    if n <= 1 {
        return areas;
    }
    let mut root: Vec<usize> = (0..n).collect();
    fn find(root: &mut [usize], mut i: usize) -> usize {
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
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

/// Seed one independent area beside the existing cave (or at the centre when
/// the board is empty). Silently skips if no clean spot is free.
#[allow(clippy::too_many_arguments)]
fn seed_single<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    builds: &mut Vec<Build>,
    kind: AreaKind,
    target: usize,
    hex_size: f64,
    rng: &mut R,
) {
    let idx = builds.len() as u32;
    let seed = pick_seed(grid, owner, builds, idx, rng);
    let Some(seed) = seed else { return };
    owner.insert(seed, idx);
    let grow = if kind == AreaKind::Dungeon {
        Grow::Shaped(new_shape(seed, true, hex_size, rng))
    } else {
        Grow::Organic { frontier: BTreeSet::new() }
    };
    let is_rect = matches!(&grow, Grow::Shaped(s) if s.is_rect());
    builds.push(Build { cells: vec![seed], kind, target, active: true, grow, is_rect });
}

/// A seed for a new independent area: the board centre for the first area,
/// otherwise a free cell two hexes out from the existing cave (so it has a
/// doorway candidate) whose whole neighbourhood is free.
fn pick_seed<R: Rng>(grid: &HexGrid, owner: &CellMap<u32>, builds: &[Build], idx: u32, rng: &mut R) -> Option<Hex> {
    if builds.is_empty() {
        let valid: Vec<Hex> = grid
            .cells()
            .iter()
            .copied()
            .filter(|&h| placeable(grid, owner, idx, h))
            .collect();
        if valid.is_empty() {
            return None;
        }
        let central: Vec<Hex> = valid.iter().copied().filter(|h| h.distance(Hex::ORIGIN) <= grid.radius / 3).collect();
        let seeds = if central.is_empty() { &valid } else { &central };
        return Some(seeds[rng.random_range(0..seeds.len())]);
    }
    let mut ring: BTreeSet<Hex> = BTreeSet::new();
    for b in builds {
        for &c in &b.cells {
            for n in c.neighbors() {
                for m in n.neighbors() {
                    ring.insert(m);
                }
            }
        }
    }
    let ring: Vec<Hex> = ring.into_iter().filter(|&h| placeable(grid, owner, idx, h)).collect();
    if ring.is_empty() {
        return None;
    }
    Some(ring[rng.random_range(0..ring.len())])
}

/// A fresh disk or (if `allow_rect`) rectangle centred on `seed`, in the
/// engine's `hex_size` pixel units.
fn new_shape<R: Rng>(seed: Hex, allow_rect: bool, hex_size: f64, rng: &mut R) -> Shape {
    let c = seed.center(hex_size);
    if allow_rect && rng.random_bool(0.5) {
        let mut order = [0, 1, 2, 3];
        order.shuffle(rng);
        Shape::Rect { c, x0: c.0, x1: c.0, y0: c.1, y1: c.1, order }
    } else {
        Shape::Disk { c, r: 0.0 }
    }
}

/// Advance one independent area by a single round's increment.
#[allow(clippy::too_many_arguments)]
fn advance_single<R: Rng>(
    grid: &HexGrid,
    owner: &mut CellMap<u32>,
    builds: &mut [Build],
    i: usize,
    params: &GrowthParams,
    hex_size: f64,
    rng: &mut R,
) {
    if builds[i].cells.len() >= builds[i].target {
        builds[i].active = false;
        return;
    }
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
                let cand: Vec<Hex> = frontier.into_iter().filter(|&h| placeable(grid, owner, i as u32, h)).collect();
                if cand.is_empty() {
                    // Seed the frontier from the current boundary once, else stop.
                    if builds[i].cells.len() == 1 {
                        let seed = builds[i].cells[0];
                        if let Grow::Organic { frontier } = &mut builds[i].grow {
                            for n in seed.neighbors() {
                                if placeable(grid, owner, i as u32, n) {
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
                    .map(|&h| {
                        let c = h.neighbors().iter().filter(|n| owner.get(**n) == Some(i as u32)).count();
                        if params.chaotic {
                            if c == 2 { 8.0 } else { 1.0 }
                        } else {
                            (c as f64).powf(params.gamma)
                        }
                    })
                    .collect();
                let pick = cand[weighted_index(rng, &weights)];
                owner.insert(pick, i as u32);
                builds[i].cells.push(pick);
                if let Grow::Organic { frontier } = &mut builds[i].grow {
                    frontier.remove(&pick);
                    for n in pick.neighbors() {
                        if placeable(grid, owner, i as u32, n) {
                            frontier.insert(n);
                        }
                    }
                }
                added += 1;
            }
        }
        Grow::Shaped(shape) => {
            // Rects reshuffle their side order each round for balanced growth.
            if let Shape::Rect { order, .. } = shape {
                order.shuffle(rng);
            }
            let cands = shape.candidates(grid, hex_size);
            let mut grew = false;
            for (cells, next) in cands {
                if !cells.is_empty() && cells.iter().all(|&c| placeable(grid, owner, i as u32, c)) {
                    for &c in &cells {
                        owner.insert(c, i as u32);
                    }
                    builds[i].cells.extend(cells);
                    builds[i].grow = Grow::Shaped(next);
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
    if let Shape::Rect { order, .. } = &mut orbits[o].shape {
        order.shuffle(rng);
    }
    let cands = orbits[o].shape.candidates(grid, hex_size);
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
        let seeds: Vec<Hex> = plan.xforms.iter().map(|xf| xf.cell(centre, gen_seed)).collect();
        // All seeds distinct, placeable, and pairwise non-adjacent.
        let uniq: std::collections::HashSet<Hex> = seeds.iter().copied().collect();
        if uniq.len() != seeds.len() {
            continue;
        }
        let base = builds.len() as u32;
        let ok = seeds.iter().enumerate().all(|(k, &c)| placeable(grid, owner, base + k as u32, c))
            && seeds.iter().enumerate().all(|(k, &c)| {
                seeds.iter().enumerate().all(|(k2, &c2)| k == k2 || c.distance(c2) >= 2)
            });
        if !ok {
            continue;
        }
        // Commit: generator first, then siblings.
        let shape = new_shape(gen_seed, allow_rect, hex_size, rng);
        let is_rect = shape.is_rect();
        let mut members = Vec::new();
        for (k, &c) in seeds.iter().enumerate() {
            let idx = builds.len();
            owner.insert(c, idx as u32);
            builds.push(Build {
                cells: vec![c],
                kind: AreaKind::Dungeon,
                target,
                active: k == 0, // only the generator's `active` matters (orbit drives)
                grow: Grow::Managed,
                is_rect,
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
        let shape = (b.kind == AreaKind::Dungeon).then(|| derive_shape(&b.cells, b.is_rect, hex_size));
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
