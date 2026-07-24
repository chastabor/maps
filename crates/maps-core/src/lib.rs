//! maps-core: procedural cave (and, later, glade/forest) map generator in
//! the style of watabou's Cave/Glade Generator. Pure and deterministic:
//! `(seed, tags)` in, map out. No I/O, no platform dependencies — usable
//! natively and from wasm.

pub mod decor;
pub mod doorway;
pub mod grid;
pub mod growth;
pub mod naming;
pub mod outline;
pub mod render;
pub mod ruins;
pub mod symmetry;
pub mod tags;
pub mod topology;
pub mod water;

use grid::HexGrid;
use growth::{Areas, GrowthParams, grid_radius, grow_areas, resolve};
use rand::Rng;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_pcg::Pcg64;
use outline::{OutlineParams, Point, build_outline};
use tags::Tags;
use topology::Topology;

/// What the generated space represents: a cave system (walls, hatching) or
/// a forest glade (clearings ringed by trees).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Cave,
    Forest,
}

/// The grid overlay drawn on the floor: the native hex lattice, a square
/// grid sized so its lines meet the hex centres of every other row, or no
/// grid at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GridStyle {
    #[default]
    Hex,
    Square,
    None,
}

/// The architectural state of an area, on the organic → ruin → dungeon
/// spectrum. `ruins_level` sets how much of the map leaves `Organic`; of that
/// geometric remainder, `dungeon_level` promotes a fraction from `Ruin`
/// (weathered geometry) to `Dungeon` (clean walls, doors, symmetry).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AreaKind {
    #[default]
    Organic,
    Ruin,
    Dungeon,
}

impl AreaKind {
    /// Whether areas of these two kinds may sit cell-adjacent with no rock gap
    /// between them. Only two **same-kind geometric** areas fuse — two dungeon
    /// rooms into one compound room (e.g. a rectangle with an attached circular
    /// silo), or two ruins into one compound ruin. Every other pair (cross-kind
    /// or anything touching organic) keeps the one-cell doorway gap. Whether a
    /// *particular* eligible pair actually fuses is gated per-area by the
    /// fusible flag (see `GenOptions::fuse_level`).
    pub fn may_fuse(self, other: AreaKind) -> bool {
        self == other && matches!(self, AreaKind::Dungeon | AreaKind::Ruin)
    }
}

/// How one door onto a dungeon room is drawn. The three leaf styles are a
/// hex-aligned bar across the doorway with jamb caps at each end: `Wood` is a
/// plain leaf, `Metal` adds a reinforcing band down its length, `Portcullis`
/// is a row of bars (drawn as circles) instead of a leaf. `Open` draws no
/// glyph at all — the doorway stays a plain framed gap (not every room-to-
/// room opening has a door). One per `topology` door; entries for doors that
/// touch no dungeon area are unused.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DoorStyle {
    #[default]
    Wood,
    Metal,
    Portcullis,
    Open,
}

pub struct CaveMap {
    /// The master seed the sub-seeds were derived from.
    pub seed: u64,
    /// Grid overlay style for rendering.
    pub grid_style: GridStyle,
    /// The three effective sub-seeds; quote these to replicate or remix the
    /// map ([`GenOptions::shape_seed`] and friends).
    pub shape_seed: u64,
    pub decor_seed: u64,
    pub name_seed: u64,
    pub mode: Mode,
    pub tags: Tags,
    pub params: GrowthParams,
    pub grid: HexGrid,
    pub areas: Areas,
    pub topology: Topology,
    /// Smoothed floor boundary loops (outer walls and interior pillars).
    pub outline: Vec<Vec<Point>>,
    /// The spliced dungeon wall runs as polylines of `(point, owning shape)`,
    /// one run per gap-bounded stretch of wall (a closed run repeats its first
    /// point). Each vertex carries the room shape it projects onto, so a run
    /// can cross the seam between two fused rooms and still offset correctly —
    /// a fused compound is one continuous band, not two capsules that notch at
    /// the seam. At render time each run is offset inward per-vertex on its
    /// shape and stroked thick — the wall the outline traced, its outer face on
    /// the traced boundary, with gaps only at doorway and exit openings.
    pub dungeon_walls: Vec<Vec<(Point, ruins::RuinShape)>>,
    /// Smoothed water pool loops at the waterline.
    pub water: Vec<Vec<Point>>,
    /// Deep-water band inside the pools (terrain well below the level).
    pub deep_water: Vec<Vec<Point>>,
    /// Mud fringe just above the waterline, ringing the pools.
    pub mud: Vec<Vec<Point>>,
    /// Rubble stone polygons.
    pub stones: Vec<Vec<Point>>,
    /// Wall hatching: five-stroke cone fans along the wall, each with an
    /// opaque footprint that hides fans beneath it; clipped by the floor at
    /// render time (cave mode).
    pub hatching: Vec<decor::HatchFan>,
    /// Border tree canopies as star polygons with a depth band
    /// (0 = nearest the clearing, rendered lightest) (forest mode).
    pub trees: Vec<(Vec<Point>, usize)>,
    /// Faded stipple dots along ruin walls as (centre, radius, opacity)
    /// discs — larger and darker near the wall (cave mode).
    pub dots: Vec<(Point, f64, f64)>,
    /// Masonry tiles along ruin walls (forest mode).
    pub tiles: Vec<Vec<Point>>,
    /// Per-area geometric ruin shape, if the area was reshaped.
    pub ruins: Vec<Option<ruins::RuinShape>>,
    /// Per-door glyph style, aligned with `topology.doors`. Only doors that
    /// touch a `Dungeon` area are rendered; the rest carry default entries.
    /// (Per-area kinds live on `areas` — see [`growth::Areas::kind`].)
    pub door_styles: Vec<DoorStyle>,
    /// Doorway mouths onto dungeon rooms (clustered doors + lip geometry);
    /// the outline's door plugs and the rendered door glyphs both follow
    /// these.
    pub mouths: Vec<doorway::Mouth>,
    /// Floor tile pattern elements on ruin-area cells (pattern tag).
    pub floor_pattern: Vec<decor::PatternElem>,
    pub title: String,
}

impl CaveMap {
    /// Whether the area at `i` is a clean, doored dungeon room (as opposed to
    /// organic or weathered ruin). Out-of-range indices are not dungeons.
    pub fn is_dungeon(&self, i: usize) -> bool {
        self.areas.kinds().get(i) == Some(&AreaKind::Dungeon)
    }
}

/// Cells forming the "neck" of every narrowly-fused dungeon pair: two dungeon
/// rooms that grew cell-adjacent but touch across only one or two faces. The
/// neck is the touching cells of both rooms. The outline locks these on their
/// raw hex corners (rather than projecting each side onto its own pinching room
/// wall), so the join is a full-hex-width, hex-aligned neck — the two touching
/// hexes are already floor, so nothing new is filled. Rooms touching across ≥3
/// faces already read as one compound and contribute nothing.
fn fused_necks(areas: &Areas) -> std::collections::HashSet<grid::Hex> {
    use std::collections::{HashMap, HashSet};
    let n = areas.count();
    let is_d = |i: usize| areas.kind(i) == AreaKind::Dungeon;
    // Touching cells per dungeon pair; the (cell, foreign-neighbour) adjacency
    // count is twice the seam's face width (each face is seen from both sides).
    let mut seam: HashMap<(usize, usize), (Vec<grid::Hex>, usize)> = HashMap::new();
    for a in 0..n {
        if !is_d(a) {
            continue;
        }
        for &h in &areas.cells[a] {
            for nb in h.neighbors() {
                if let Some(b) = areas.owner_of(nb).filter(|&b| b != a && is_d(b)) {
                    let e = seam.entry((a.min(b), a.max(b))).or_default();
                    e.0.push(h);
                    e.1 += 1;
                }
            }
        }
    }
    let mut neck = HashSet::new();
    for (cells, faces) in seam.values() {
        if faces / 2 <= 2 {
            neck.extend(cells.iter().copied()); // narrow touch: give it a neck
        }
    }
    neck
}

/// A clean, hex-aligned neck joining a fused **circle↔rectangle** pair, in
/// place of the pinched improvised join two independently-derived shapes give.
/// The two `line`s are the neck's outer walls — parallel, along the hex-edge
/// axis toward the circle (150°-family diagonal) — each `(circle_arc_hit,
/// rectangle_anchor)`; one anchored at the rectangle's near corner, the other
/// at the connecting wall-hex's far point (one hex row over). `hall` is a
/// `StraightHall` spanning between them so the renderer's per-vertex inner
/// offset draws the neck's inner wall.
struct Neck {
    circ: ruins::RuinShape,
    rect: ruins::RuinShape,
    lines: [(Point, Point); 2],
    hall: ruins::RuinShape,
}

/// One [`Neck`] per fused circle↔rectangle pair (see [`splice_necks`]).
fn circle_rect_necks(areas: &Areas, s: f64) -> Vec<Neck> {
    use ruins::RuinShape;
    let mut necks = Vec::new();
    let n = areas.count();
    for a in 0..n {
        for b in 0..n {
            // a = rectangle, b = circle.
            let (Some(rect @ RuinShape::Rect { cx: rcx, cy: rcy, hw: rhw, hh: rhh }), Some(circ @ RuinShape::Circle { cx: ccx, cy: ccy, r: cr })) =
                (areas.shape(a), areas.shape(b))
            else {
                continue;
            };
            // Only fused pairs touch (everyone else keeps a rock gap).
            if !areas.cells[a].iter().any(|h| h.neighbors().iter().any(|nb| areas.owner_of(*nb) == Some(b))) {
                continue;
            }
            // Interior (non-border) rows/columns of each shape — a cell is
            // interior when all its neighbours share the area, so the wall
            // doesn't cut it. A shared interior ROW means a straight horizontal
            // corridor could join the two (a shared COLUMN → a vertical one);
            // those clean alignments get a straight path (deferred). The angle
            // neck is only for CORNER fusions, where neither axis aligns.
            let interior = |i: usize| {
                let (mut rows, mut cols) = (std::collections::HashSet::new(), std::collections::HashSet::new());
                for &h in &areas.cells[i] {
                    if h.neighbors().iter().all(|nb| areas.owner_of(*nb) == Some(i)) {
                        rows.insert(h.r);
                        cols.insert(h.q);
                    }
                }
                (rows, cols)
            };
            let (r_rows, r_cols) = interior(a);
            let (c_rows, c_cols) = interior(b);
            if r_rows.intersection(&c_rows).next().is_some()
                || r_cols.intersection(&c_cols).next().is_some()
            {
                continue; // horizontal/vertical alignment → straight path (deferred)
            }
            // The geometry below assumes the circle sits beyond a left/right
            // edge (its offset is x-dominant). A corner where the circle is
            // beyond a top/bottom edge needs the transposed construction —
            // deferred, since pointy-top top/bottom edges differ from sides.
            if ((ccx - rcx) / rhw).abs() < ((ccy - rcy) / rhh).abs() {
                continue;
            }
            // The rectangle's wall-cell adjacent to the circle, on the edge
            // facing it (its centre sits on that edge).
            let sgnx = if ccx < rcx { -1.0 } else { 1.0 };
            let near_x = rcx + sgnx * rhw;
            let Some(conn) = areas.cells[a]
                .iter()
                .filter(|h| h.neighbors().iter().any(|nb| areas.owner_of(*nb) == Some(b)))
                .copied()
                .min_by(|p, q| {
                    (p.center(s).0 - near_x).abs().total_cmp(&(q.center(s).0 - near_x).abs())
                })
            else {
                continue;
            };
            let sgny = if ccy < rcy { -1.0 } else { 1.0 };
            let cc = conn.center(s);
            // Anchors on the rectangle's near edge: the near corner on the
            // circle's vertical side, and the connecting hex's opposite point.
            let corner = (near_x, rcy + sgny * rhh);
            let hex_pt = (cc.0, cc.1 - sgny * s);
            // Neck direction: the hex-edge diagonal pointing at the circle.
            let dir = (sgnx * crate::grid::SQRT3 / 2.0, sgny * 0.5);
            // Extend each anchor along `dir` to the circle's outer arc.
            let hit = |p: Point| -> Option<Point> {
                let (ex, ey) = (p.0 - ccx, p.1 - ccy);
                let bq = ex * dir.0 + ey * dir.1;
                let cq = ex * ex + ey * ey - cr * cr;
                let disc = bq * bq - cq;
                (disc >= 0.0).then(|| -bq - disc.sqrt()).filter(|&t| t > 0.0).map(|t| (p.0 + t * dir.0, p.1 + t * dir.1))
            };
            let (Some(c_hit), Some(h_hit)) = (hit(corner), hit(hex_pt)) else { continue };
            // A `StraightHall` whose two sides are the neck walls: centreline
            // midway between them, half-width the perpendicular half-distance.
            let mid0 = ((corner.0 + hex_pt.0) / 2.0, (corner.1 + hex_pt.1) / 2.0);
            let mid1 = ((c_hit.0 + h_hit.0) / 2.0, (c_hit.1 + h_hit.1) / 2.0);
            let nrm = (-dir.1, dir.0);
            let hw = ((corner.0 - hex_pt.0) * nrm.0 + (corner.1 - hex_pt.1) * nrm.1).abs() / 2.0;
            let hall = RuinShape::StraightHall { ax: mid0.0, ay: mid0.1, bx: mid1.0, by: mid1.1, hw };
            necks.push(Neck { circ, rect, lines: [(c_hit, corner), (h_hit, hex_pt)], hall });
        }
    }
    necks
}

/// Splice each [`Neck`] into the `dungeon_walls` band: in the merged compound
/// run, the two seam crossings (where circle vertices meet rectangle vertices,
/// the pinch) are replaced by the neck's outer wall line for that side, tagged
/// with the hall so the renderer offsets its inner wall. The band then flows
/// circle arc → neck → rectangle wall as one continuous wall.
fn splice_necks(walls: &mut [Vec<(Point, ruins::RuinShape)>], necks: &[Neck]) {
    let dist = |a: Point, b: Point| (a.0 - b.0).hypot(a.1 - b.1);
    for neck in necks {
        let ruins::RuinShape::StraightHall { ax, ay, bx, by, hw } = neck.hall else { continue };
        let (a, len, dir, nrm) = {
            let d = (bx - ax, by - ay);
            let l = d.0.hypot(d.1).max(1e-9);
            ((ax, ay), l, (d.0 / l, d.1 / l), (-d.1 / l, d.0 / l))
        };
        // Point projected onto the hall: (distance along axis, signed perp).
        let proj = |p: Point| {
            ((p.0 - a.0) * dir.0 + (p.1 - a.1) * dir.1, (p.0 - a.0) * nrm.0 + (p.1 - a.1) * nrm.1)
        };
        let in_neck = |p: Point| {
            let (t, pp) = proj(p);
            t >= -2.0 && t <= len + 2.0 && pp.abs() <= hw + 2.0
        };
        // Which line lies on which perpendicular side.
        let side = |l: &(Point, Point)| {
            proj(((l.0.0 + l.1.0) / 2.0, (l.0.1 + l.1.1) / 2.0)).1.signum()
        };
        for run in walls.iter_mut() {
            if !run.iter().any(|v| v.1 == neck.circ) || !run.iter().any(|v| v.1 == neck.rect) {
                continue;
            }
            let closed = run.len() > 2 && run.first().map(|v| v.0) == run.last().map(|v| v.0);
            let core = if closed { &run[..run.len() - 1] } else { &run[..] };
            // Rotate so index 0 is a kept (non-neck) vertex, else a neck run
            // wrapping the seam would be split.
            let Some(start) = core.iter().position(|v| !in_neck(v.0)) else { continue };
            let n = core.len();
            let mut out: Vec<(Point, ruins::RuinShape)> = Vec::with_capacity(n + 4);
            let mut k = 0;
            while k < n {
                let v = core[(start + k) % n];
                if in_neck(v.0) {
                    let s = proj(v.0).1.signum();
                    let line = if side(&neck.lines[0]) == s { neck.lines[0] } else { neck.lines[1] };
                    while k < n && in_neck(core[(start + k) % n].0) {
                        k += 1;
                    }
                    // Order the line's two ends to continue the polyline.
                    let prev = out.last().map(|v| v.0).unwrap_or(line.0);
                    let (e0, e1) = if dist(prev, line.0) <= dist(prev, line.1) {
                        (line.0, line.1)
                    } else {
                        (line.1, line.0)
                    };
                    out.push((e0, neck.hall));
                    out.push((e1, neck.hall));
                } else {
                    out.push(v);
                    k += 1;
                }
            }
            if closed {
                if let Some(&first) = out.first() {
                    out.push(first);
                }
            }
            *run = out;
        }
    }
}

/// Everything that shapes generation besides the seed.
#[derive(Clone, Debug, Default)]
pub struct GenOptions {
    pub mode: Mode,
    /// Grid overlay style (default: hex).
    pub grid: GridStyle,
    /// `None` picks random tags from the seed.
    pub tags: Option<Tags>,
    pub outline: OutlineParams,
    /// Water level as a fill fraction in 0..=1: 0 is completely dry, 0.5
    /// floods the lowest half of the terrain, 1 submerges everything.
    /// Fine-tunes the water tag's default (wet 0.45, untagged 0.15); the
    /// `dry` tag always means no water and ignores this.
    pub water_level: Option<f64>,
    /// Fraction (0..=1) of the non-corridor areas that take on geometric
    /// ruin shapes (rectangles/circles) in place of their organic outline.
    /// Fine-tunes the ruins tag's default (ruins 0.5, untagged 0.1); the
    /// `organic` tag always means no ruins and ignores this.
    pub ruins_level: Option<f64>,
    /// Fraction (0..=1) of the geometric (ruin) areas promoted to clean
    /// **dungeon** rooms — crisp walls, rendered doors, and (later) symmetric
    /// wings — instead of weathered ruins. Nested inside `ruins_level`: with
    /// no geometric areas it does nothing. Fine-tunes the dungeon tag's
    /// default (dungeon 0.6, untagged 0.0); the `natural` tag forces 0.
    pub dungeon_level: Option<f64>,
    /// Probability (0..=1) that each geometric area (dungeon or ruin) is marked
    /// **fusible** at classification. Two fusible same-kind areas that grow into
    /// each other fuse into a compound (shared edge, no doorway) instead of
    /// stopping at the rock gap. Fine-tunes the fuse tag's default (fused 0.71,
    /// untagged 0.0); the `separate` tag forces 0.
    pub fuse_level: Option<f64>,
    /// Override the shape stream (tags, areas, topology, outline, water,
    /// stones). Defaults to a sub-seed derived from the master seed.
    pub shape_seed: Option<u64>,
    /// Override the decoration stream (hatch fans / tree canopies).
    pub decor_seed: Option<u64>,
    /// Override the naming stream (the title).
    pub name_seed: Option<u64>,
    /// Use this exact title instead of generating one (empty/whitespace is
    /// ignored). The name stream is left untouched, so seeded naming
    /// resumes when the override is removed.
    pub title: Option<String>,
}

/// Generate a cave map. `tags: None` picks random tags from the seed.
pub fn generate(seed: u64, tags: Option<Tags>) -> CaveMap {
    generate_with(
        seed,
        &GenOptions {
            tags,
            ..GenOptions::default()
        },
    )
}

/// The tags a master seed rolls when none are supplied — the same
/// derivation `generate_with` uses, exposed so UIs can preview/edit them.
pub fn random_tags_for(seed: u64) -> Tags {
    let shape_seed = sub_seed(seed, 0);
    let mut tag_rng = Pcg64::seed_from_u64(sub_seed(shape_seed, 3));
    Tags::random(&mut tag_rng)
}

/// Derive a stream-specific sub-seed from the master seed (splitmix64).
fn sub_seed(seed: u64, stream: u64) -> u64 {
    let mut z = seed.wrapping_add((stream + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Assign a kind (and a fusible flag) to each of the `n` area slots *before*
/// growth, so growth can react. `round(ruins_level·n)` slots become geometric
/// (ruin ∪ dungeon); of those, `round(dungeon_level·n_geo)` are dungeon and the
/// rest ruin; all remaining slots are organic. Which slots take which kind is a
/// shape-stream shuffle. Each geometric slot is independently marked **fusible**
/// with probability `fuse_level` — two fusible same-kind areas that grow into
/// each other fuse (see `AreaKind::may_fuse`); organic slots are never fusible.
/// With no geometric areas it draws nothing, so a fully-organic map's growth is
/// untouched.
fn classify_slots<R: Rng>(
    n: usize,
    ruins_level: f64,
    dungeon_level: f64,
    fuse_level: f64,
    rng: &mut R,
) -> (Vec<AreaKind>, Vec<bool>) {
    let n_geo = ((ruins_level.clamp(0.0, 1.0) * n as f64).round() as usize).min(n);
    if n_geo == 0 {
        return (vec![AreaKind::Organic; n], vec![false; n]);
    }
    let n_dun = ((dungeon_level.clamp(0.0, 1.0) * n_geo as f64).round() as usize).min(n_geo);
    let mut slots: Vec<usize> = (0..n).collect();
    slots.shuffle(rng);
    let mut kinds = vec![AreaKind::Organic; n];
    for &s in &slots[..n_dun] {
        kinds[s] = AreaKind::Dungeon;
    }
    for &s in &slots[n_dun..n_geo] {
        kinds[s] = AreaKind::Ruin;
    }
    // Fusible marking, on the geometric slots only, in slot order for a stable
    // draw independent of the shuffle above. Skipped entirely (drawing no
    // randomness) when fusion is off, so a map without fusion is byte-for-byte
    // what it was before this feature existed.
    let p = fuse_level.clamp(0.0, 1.0);
    let fusible: Vec<bool> = if p <= 0.0 {
        vec![false; n]
    } else {
        (0..n)
            .map(|s| kinds[s] != AreaKind::Organic && rng.random_bool(p))
            .collect()
    };
    (kinds, fusible)
}

/// Generate a map with explicit options.
///
/// Randomness is split into three independent streams so each can be
/// re-rolled without disturbing the others: shape, decoration and name.
/// With all three (or just the master seed) plus the same tags/options, the
/// image replicates identically anywhere.
pub fn generate_with(seed: u64, opts: &GenOptions) -> CaveMap {
    let mode = opts.mode;
    let oparams = &opts.outline;
    let shape_seed = opts.shape_seed.unwrap_or_else(|| sub_seed(seed, 0));
    let decor_seed = opts.decor_seed.unwrap_or_else(|| sub_seed(seed, 1));
    let name_seed = opts.name_seed.unwrap_or_else(|| sub_seed(seed, 2));

    // Random tags come from their own stream (derived from the shape seed)
    // so that supplying the same tags explicitly — as a replica must —
    // leaves the shape stream untouched and reproduces the identical map.
    let tags = opts.tags.clone().unwrap_or_else(|| {
        let mut tag_rng = Pcg64::seed_from_u64(sub_seed(shape_seed, 3));
        Tags::random(&mut tag_rng)
    });
    let mut rng = Pcg64::seed_from_u64(shape_seed);
    let params = resolve(&tags, &mut rng);
    // The tag picks the state; the level only fine-tunes it: organic always
    // means no ruins, while ruins/untagged use the override in place of
    // their default fraction.
    let ruins_level = match tags.ruins {
        Some(tags::RuinsTag::Organic) => 0.0,
        Some(tags::RuinsTag::Ruins) => opts.ruins_level.unwrap_or(0.5),
        None => opts.ruins_level.unwrap_or(0.1),
    };
    // Nested inside ruins: the fraction of the geometric areas that become
    // clean dungeon rooms rather than weathered ruins. `natural` forces none;
    // untagged is none unless overridden.
    let dungeon_level = match tags.dungeon {
        Some(tags::DungeonTag::Natural) => 0.0,
        Some(tags::DungeonTag::Dungeon) => opts.dungeon_level.unwrap_or(0.6),
        None => opts.dungeon_level.unwrap_or(0.0),
    };
    // Per-area chance of being fusible: `separate` forces none, `fused` uses the
    // default fraction, untagged is none unless overridden.
    let fuse_level = match tags.fuse {
        Some(tags::FuseTag::Separate) => 0.0,
        Some(tags::FuseTag::Fused) => opts.fuse_level.unwrap_or(0.71),
        None => opts.fuse_level.unwrap_or(0.0),
    };
    // Classify each area SLOT before growth, so growth can react to it (which
    // slots are geometric, and which of those are fusible). Classification,
    // growth, reshaping and doors all draw from the one shape stream —
    // determinism is per-seed, with no byte-compatibility to older output.
    let (slot_kinds, slot_fusible) =
        classify_slots(params.sizes.len(), ruins_level, dungeon_level, fuse_level, &mut rng);
    let grid = HexGrid::hexagon(grid_radius(&params));
    // Staggered simultaneous growth: dungeon rooms grow as their geometry and
    // symmetric wings grow as lockstep sibling orbits (symmetry is chosen
    // inside, from the shape stream).
    let mut areas =
        grow_areas(&grid, &mut rng, &params, &slot_kinds, &slot_fusible, oparams.hex_size);
    let topology = topology::build(&grid, &mut areas, &tags, oparams.hex_size, &mut rng);
    // Reshape the ruin areas to their rasterized geometry, so all downstream
    // layers (outline, water, stones, decor) see the real footprint and
    // touching shapes union at the cell level. Ruins that can't reshape are
    // demoted back to organic inside; dungeon rooms grew as their geometry.
    ruins::build(&mut areas, &topology, oparams.hex_size, &mut rng);
    let mut ruin_map = ruins::ruin_cell_map(&areas, oparams.hex_size);

    // Doorway mouths onto dungeon rooms: flush openings cut into the exact
    // wall, nothing built outside it. Only each dungeon exit's stub cells
    // still project onto a straight throat through the wall. `clean_shapes`
    // is the wall geometry (rooms, exit lips) whose decor stays a clean
    // line.
    let mouths = doorway::mouths(&topology, &areas, oparams.hex_size);
    let (plug_cells, clean_shapes) =
        doorway::apply_plugs(&mut ruin_map, &topology, &areas, oparams.hex_size);

    // Style each door that opens onto a dungeon room; other doors keep a
    // default entry and draw nothing. A door directly between two dungeon
    // rooms is sometimes left `Open` — a framed gap with no leaf; not every
    // room needs a door.
    let door_styles: Vec<DoorStyle> = topology
        .doors
        .iter()
        .map(|d| {
            let (ka, kb) = (areas.kind(d.a), areas.kind(d.b));
            if ka == AreaKind::Dungeon || kb == AreaKind::Dungeon {
                let room_to_room = ka == AreaKind::Dungeon && kb == AreaKind::Dungeon;
                let roll = rng.random_range(0..100);
                if room_to_room {
                    // 15% open; the leaf styles keep their relative shares.
                    match roll {
                        0..15 => DoorStyle::Open,
                        15..34 => DoorStyle::Metal,
                        34..47 => DoorStyle::Portcullis,
                        _ => DoorStyle::Wood,
                    }
                } else {
                    match roll {
                        0..22 => DoorStyle::Metal,
                        22..37 => DoorStyle::Portcullis,
                        _ => DoorStyle::Wood,
                    }
                }
            } else {
                DoorStyle::default()
            }
        })
        .collect();
    // Every dungeon-area cell, mapped to its room's shape: the outline
    // splices these boundary runs onto the exact geometry (even cells
    // `ruin_cell_map` excludes as contested), and the set keeps them out of
    // the weathered ruin decor (stipple / masonry) below.
    let dungeon_cells: std::collections::HashMap<grid::Hex, ruins::RuinShape> = (0..areas.count())
        .filter(|&i| areas.kind(i) == AreaKind::Dungeon)
        .filter_map(|i| areas.shapes()[i].map(|sh| (i, sh)))
        .flat_map(|(i, sh)| areas.cells[i].iter().map(move |&c| (c, sh)))
        .collect();
    // Narrow fused seams: two dungeon rooms that grew cell-adjacent but touch
    // across only one or two cell-faces pinch to a thin waist when each side
    // projects onto its own room wall. Collect the touching cells (and the
    // unowned rock pockets flanking them) as a "neck": the outline locks these
    // on their raw hex corners instead — a full-width, hex-aligned connection
    // between the two rooms (see `smooth_loops`). Wide touches (≥3 faces)
    // already read as one compound and are left alone.
    let neck_cells = fused_necks(&areas);
    let jambs = doorway::jambs(&mouths, &topology, &areas, oparams.hex_size);
    let (outline, mut dungeon_walls) =
        build_outline(&areas, &topology, &ruin_map, &dungeon_cells, &neck_cells, &jambs, oparams, &mut rng);
    // A fused circle+rectangle joins with a clean hex-aligned neck: splice the
    // neck's outer walls into the band in place of the pinched seam.
    splice_necks(&mut dungeon_walls, &circle_rect_necks(&areas, oparams.hex_size));
    let w = water::build_water(&areas, &topology, oparams, &tags, opts.water_level, &mut rng);
    let (floor, narrow) = outline::floor_and_narrow(&areas, &topology);
    let stones = decor::stones(&floor, &narrow, &w.cells, oparams.hex_size, &mut rng);

    // Only weathered (ruin) walls get stipple/masonry; dungeon and doorway
    // plug cells are excluded here. Clean walls (rooms, lips) skip the
    // organic hatching too, classified against `clean_shapes` geometry; the
    // rest of an exit stub hatches like any passage.
    let ruin_cells: std::collections::HashSet<grid::Hex> = ruin_map
        .keys()
        .copied()
        .filter(|h| !dungeon_cells.contains_key(h) && !plug_cells.contains(h))
        .collect();

    let mut decor_rng = Pcg64::seed_from_u64(decor_seed);
    let (hatching, dots, trees, tiles) = match mode {
        Mode::Cave => {
            let (fans, dots) = decor::hatching(
                &outline,
                &ruin_cells,
                &clean_shapes,
                oparams.hex_size,
                &mut decor_rng,
            );
            (fans, dots, Vec::new(), Vec::new())
        }
        Mode::Forest => {
            let (trees, tiles) = decor::trees(
                &outline,
                &ruin_cells,
                &clean_shapes,
                oparams.hex_size,
                &mut decor_rng,
            );
            (Vec::new(), Vec::new(), trees, tiles)
        }
    };
    // Floor tiles on every geometric area — weathered ruins and clean dungeon
    // rooms alike — after the other decor so `plain` maps keep their exact
    // output. One sorted cell list per area.
    let pattern_tag = tags.pattern.unwrap_or(tags::PatternTag::Plain);
    let ruin_area_cells: Vec<Vec<grid::Hex>> = (0..areas.count())
        .filter(|&i| areas.kind(i) != AreaKind::Organic)
        .map(|i| {
            let mut v = areas.cells[i].clone();
            v.sort_unstable();
            v
        })
        .collect();
    let floor_pattern =
        decor::floor_pattern(&ruin_area_cells, pattern_tag, oparams.hex_size, &mut decor_rng);

    // Snapshot the shapes before `areas` moves into the map.
    let ruin_shapes = areas.shapes().to_vec();
    let title = match opts.title.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            let mut name_rng = Pcg64::seed_from_u64(name_seed);
            naming::title(&mut name_rng, !w.pools.is_empty(), mode)
        }
    };
    CaveMap {
        seed,
        grid_style: opts.grid,
        shape_seed,
        decor_seed,
        name_seed,
        mode,
        tags,
        params,
        grid,
        areas,
        topology,
        outline,
        dungeon_walls,
        water: w.pools,
        deep_water: w.deep,
        mud: w.mud,
        stones,
        hatching,
        trees,
        dots,
        tiles,
        ruins: ruin_shapes,
        door_styles,
        mouths,
        floor_pattern,
        title,
    }
}
