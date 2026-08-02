//! Boundary tracing and smoothing: converts the cell-based cave floor into
//! organic closed curves, erasing all traces of the hex grid.
//!
//! Pipeline (after watabou, via Boris the Brave's analysis): trace boundary
//! loops -> subdivide edges -> Laplacian smoothing (bumpiness) -> pull
//! tunnel vertices toward cell centres (narrowing) -> random vertex offsets
//! (irregularity) -> two rounds of subdivide + finer jitter (roughness) ->
//! Chaikin corner cutting.

use crate::doorway::Jamb;
use crate::grid::Hex;
use crate::growth::Areas;
use crate::ruins::RuinShape;
use crate::topology::Topology;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub use crate::geom::Point;

/// One dungeon wall run: a polyline of `(point, owning shape)`, closed runs
/// repeating their first point. Each vertex carries the shape it projects onto, so a
/// run can cross the seam between two fused rooms and still offset correctly.
pub type WallRun = Vec<(Point, RuinShape)>;

/// What the smoothing passes must respect on a boundary, and how.
///
/// Bundled because these four always travel together and mean nothing apart: they are
/// the answer to "which of these vertices are not free to move". Water and mud
/// boundaries have none of them — they are purely organic — so most callers pass one
/// value built once.
#[derive(Clone, Copy)]
pub struct Constraints<'a> {
    /// Every ruin cell's geometric shape. Those vertices project onto the shape and
    /// lock against all jitter, keeping the wall crisp.
    pub ruin_cells: &'a HashMap<Hex, RuinShape>,
    /// Every dungeon room cell's shape. Those boundary runs are replaced wholesale by
    /// the room's exact wall — see `splice_dungeon_runs` — and locked from the start.
    pub dungeon_cells: &'a HashMap<Hex, RuinShape>,
    /// Fusion-seam and corridor cells, locked on their own raw hex boundary rather
    /// than projected onto either room's shape (projecting would undo the fill).
    pub neck_cells: &'a HashSet<Hex>,
    /// Doorway jambs, which hold an opening open against the smoothing.
    pub jambs: &'a [Jamb],
}

impl Constraints<'static> {
    /// No constraints: a purely organic boundary, which is every caller outside the
    /// cave floor itself (water, mud, and the timing bench's bare floor).
    ///
    /// Shared empty statics, so a caller needs no locals of its own. Combine with
    /// struct-update syntax to constrain just one thing —
    /// `Constraints { ruin_cells: &map, ..Constraints::none() }`.
    pub fn none() -> Self {
        static SHAPES: LazyLock<HashMap<Hex, RuinShape>> = LazyLock::new(HashMap::new);
        static CELLS: LazyLock<HashSet<Hex>> = LazyLock::new(HashSet::new);
        Constraints {
            ruin_cells: &SHAPES,
            dungeon_cells: &SHAPES,
            neck_cells: &CELLS,
            jambs: &[],
        }
    }
}

/// Quantize a coordinate to an exact tenth of a pixel. All geometry stored
/// on `CaveMap` goes through this (or `quantize2` for radii), so the SVG
/// writer can print coordinates with pure integer formatting and the stored
/// values equal the rendered ones exactly.
#[inline]
pub(crate) fn quantize(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Quantize to an exact hundredth (small radii keep more precision).
#[inline]
pub(crate) fn quantize2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[inline]
pub(crate) fn quantize_pt(p: Point) -> Point {
    (quantize(p.0), quantize(p.1))
}

/// Directions indexed by centre-to-centre angle `60k` degrees (pointy-top,
/// y-down). Edge `(cell, k)` runs from corner `k` to corner `k+1`, where
/// corner `i` sits at angle `60i - 30`.
const D: [Hex; 6] = [
    Hex { q: 1, r: 0 },
    Hex { q: 0, r: 1 },
    Hex { q: -1, r: 1 },
    Hex { q: -1, r: 0 },
    Hex { q: 0, r: -1 },
    Hex { q: 1, r: -1 },
];

#[derive(Clone, Debug)]
pub struct OutlineParams {
    pub hex_size: f64,
    /// Laplacian smoothing strength, 0..1.
    pub bumpiness: f64,
    /// Number of Laplacian smoothing passes.
    pub smooth_passes: usize,
    /// Vertex jitter as a fraction of hex size.
    pub irregularity: f64,
    /// Fine jitter for the two subdivide-and-roughen rounds.
    pub roughness: f64,
    /// How far tunnel-cell vertices are pulled toward their cell centre.
    pub narrow_pull: f64,
    pub chaikin_iters: usize,
}

impl Default for OutlineParams {
    fn default() -> Self {
        OutlineParams {
            hex_size: 12.0,
            bumpiness: 0.55,
            smooth_passes: 1,
            irregularity: 0.16,
            roughness: 0.07,
            narrow_pull: 0.4,
            chaikin_iters: 2,
        }
    }
}

/// The cave floor cell set and its "narrow" subset (corridors, doors, exit
/// passages — cells whose boundary vertices get pulled inward).
pub(crate) fn floor_and_narrow(areas: &Areas, topology: &Topology) -> (HashSet<Hex>, HashSet<Hex>) {
    let mut floor: HashSet<Hex> = HashSet::new();
    let mut narrow: HashSet<Hex> = HashSet::new();
    for i in 0..areas.count() {
        for c in areas.floor_cells(i) {
            floor.insert(c);
            if topology.is_corridor[i] {
                narrow.insert(c);
            }
        }
    }
    for d in &topology.connections {
        floor.insert(d.cell());
        narrow.insert(d.cell());
    }
    // Fill the lone pillar between two merged distance-2 doors, so their one
    // wide opening is backed by continuous floor instead of a floating rock nub
    // (which the outline would weave around, crossing the two passages).
    for &(_, _, pillar) in &topology.merged_doors {
        floor.insert(pillar);
        narrow.insert(pillar);
    }
    for e in &topology.exits {
        for &c in &e.stub {
            floor.insert(c);
            narrow.insert(c);
        }
    }
    (floor, narrow)
}

/// Closed smoothed loops (outer boundaries and holes, distinguishable only
/// by winding; render with fill-rule="evenodd"). `ruin_cells` maps cells of
/// reshaped areas to their geometry (seam cells already excluded — see
/// `ruins::ruin_cell_map`). `dungeon_cells` maps every dungeon room cell to
/// its room's shape: those boundary runs are **spliced** onto the exact
/// geometry (shape-tile tracing) and locked against every pass.
/// Returns `(loops, dungeon_walls)`: the smoothed floor loops, plus each
/// spliced dungeon wall run as `(room shape, polyline)` (a closed run
/// repeats its first point) — the renderer offsets each run inward on its
/// shape and strokes it thick, so the wall band covers precisely the wall
/// that was traced and never a stretch of perimeter that another shape's
/// floor swallowed.
pub fn build_outline<R: Rng>(
    areas: &Areas,
    topology: &Topology,
    constraints: Constraints<'_>,
    params: &OutlineParams,
    rng: &mut R,
) -> (Vec<Vec<Point>>, Vec<WallRun>) {
    let (floor, narrow) = floor_and_narrow(areas, topology);
    smooth_loops(trace_loops(&floor), &narrow, constraints, params, rng)
}

/// Run any cell-set boundary through the full smoothing pipeline. Vertices
/// owned by ruin cells are projected onto their geometric shape and locked
/// against all jitter, so those wall sections stay crisp; runs owned by
/// dungeon cells are replaced wholesale by the room's exact wall (see
/// `splice_dungeon_runs`) and locked from the start.
pub(crate) fn smooth_loops<R: Rng>(
    loops: Vec<Vec<(Hex, usize)>>,
    narrow: &HashSet<Hex>,
    constraints: Constraints<'_>,
    params: &OutlineParams,
    rng: &mut R,
) -> (Vec<Vec<Point>>, Vec<WallRun>) {
    let Constraints {
        ruin_cells,
        dungeon_cells,
        neck_cells,
        jambs,
    } = constraints;
    let size = params.hex_size;
    let mut walls: Vec<WallRun> = Vec::new();
    let out = loops
        .into_iter()
        .flat_map(|lp| {
            // Tagged points: position, the owning cell's centre if that cell
            // is a narrow tunnel, its ruin shape if it has one, and whether
            // it belongs to a dungeon room. A dungeon cell's shape comes from
            // `dungeon_cells` (every room cell, even ones `ruin_cell_map`
            // excludes as contested — the splice overrides those).
            let mut pts: Vec<TaggedPoint> = lp
                .iter()
                .map(|&(cell, corner)| {
                    let p = corner_point(cell, corner, size);
                    let tag = narrow.contains(&cell).then(|| cell.center(size));
                    // A join's cells — a narrow fused seam's, and the floor a
                    // corridor claimed across a gap — carry their OWN hex
                    // boundary, so they stay on their raw hex corners (the
                    // full-width neck) instead of projecting onto either room's
                    // pinching wall. Tagging them with a real (perimeter-bearing)
                    // shape rather than `None` keeps them *splicable*, so the band
                    // merge flows through the seam and a fused pair's two rooms
                    // land in one wall run — which is what lets the fusion
                    // connectors reach them at all.
                    let dungeon_shape = dungeon_cells.get(&cell).copied();
                    if neck_cells.contains(&cell) {
                        let c = cell.center(size);
                        let hex = RuinShape::HexCell {
                            cx: c.0,
                            cy: c.1,
                            s: size,
                        };
                        return (p, tag, Some(hex), dungeon_shape.is_some());
                    }
                    let ruin = dungeon_shape.or_else(|| ruin_cells.get(&cell).copied());
                    (p, tag, ruin, dungeon_shape.is_some())
                })
                .collect();

            // Shape-tile tracing: swap each dungeon run's raster vertices for
            // the exact wall before any smoothing or random pass sees them.
            splice_dungeon_runs(&mut pts, jambs, size, &mut walls);

            pts = subdivide_tagged(&pts);
            for _ in 0..params.smooth_passes {
                smooth(&mut pts, params.bumpiness);
            }
            for (p, tag, _, dungeon) in pts.iter_mut() {
                if *dungeon {
                    continue;
                }
                if let Some(c) = tag {
                    p.0 += (c.0 - p.0) * params.narrow_pull;
                    p.1 += (c.1 - p.1) * params.narrow_pull;
                }
            }

            // Blend each ruin vertex toward its shape: full projection deep
            // inside a run of ruin cells, ramping to organic over the run's
            // last few vertices. A hard snap here lets the loop fold over
            // itself where the shape cuts inside the original wall, which
            // renders as inverted pockets and pinched passage mouths.
            const RAMP: f64 = 3.0;
            let n = pts.len();
            let mut dist = vec![u32::MAX; n];
            for (i, &(_, _, ruin, _)) in pts.iter().enumerate() {
                if ruin.is_none() {
                    dist[i] = 0;
                }
            }
            if dist.contains(&0) {
                // Cyclic distance to the nearest organic vertex.
                for _ in 0..2 {
                    for i in 0..n {
                        let d = dist[(i + n - 1) % n].saturating_add(1);
                        dist[i] = dist[i].min(d);
                    }
                    for i in (0..n).rev() {
                        let d = dist[(i + 1) % n].saturating_add(1);
                        dist[i] = dist[i].min(d);
                    }
                }
            }
            // Dungeon wall vertices project HARD onto their room's exact
            // geometry (no organic ramp) and stay locked against every later
            // pass; ruin vertices blend as before. A dungeon vertex without a
            // shape (seam/contested cells of a fused compound) stays locked
            // on the raw traced hex boundary instead.
            let mut locked: Vec<bool> = pts.iter().map(|&(_, _, _, dungeon)| dungeon).collect();
            for i in 0..n {
                if let Some(shape) = pts[i].2 {
                    let w_run = if pts[i].3 || dist[i] == u32::MAX {
                        1.0
                    } else {
                        (dist[i] as f64 / RAMP).min(1.0)
                    };
                    let p = pts[i].0;
                    let proj = shape.project(p);
                    // Halls project each vertex to the *nearest* wall, so a
                    // vertex far from both walls (a radial side wall, or the
                    // far side of a two-cell-wide raster band) would jump
                    // across the passage and can land on the same wall as
                    // the opposite side — the pinch. Fade the projection out
                    // with displacement: crisp within half a cell, organic
                    // beyond 1.5 cells. Rooms are convex with a coverage-
                    // filtered raster, so their pull-in is always fold-safe.
                    let w_disp = match shape {
                        // `HexCell` joins the rooms: it is convex, and a neck
                        // vertex already lies on one of its corners, so the
                        // projection is a no-op and locking it is exactly the
                        // raw-hex lock the neck wants.
                        RuinShape::Rect { .. }
                        | RuinShape::Circle { .. }
                        | RuinShape::HexCell { .. } => 1.0,
                        // A connector's trapezoid belongs with the halls: same soft
                        // displacement, since it is a corridor wall, not a room perimeter.
                        RuinShape::StraightHall { .. }
                        | RuinShape::Trapezoid { .. }
                        | RuinShape::ArcHall { .. } => {
                            let d = (proj.0 - p.0).hypot(proj.1 - p.1);
                            ((1.5 * size - d) / size).clamp(0.0, 1.0)
                        }
                    };
                    let w = w_run * w_disp;
                    pts[i].0 = (p.0 + (proj.0 - p.0) * w, p.1 + (proj.1 - p.1) * w);
                    locked[i] = w >= 0.999;
                }
            }

            // Locked (fully projected) vertices skip every jitter pass;
            // midpoints stay locked only when both ends are, so transitions
            // to organic wall loosen up naturally.
            let mut plain: Vec<(Point, bool)> = pts
                .into_iter()
                .zip(locked)
                .map(|((p, _, _, _), l)| (p, l))
                .collect();
            jitter_unlocked(&mut plain, params.irregularity * size, rng);
            for round in 0..2 {
                plain = subdivide_locked(&plain);
                let mag = params.roughness * size / (round + 1) as f64;
                jitter_unlocked(&mut plain, mag, rng);
            }
            // Lock-aware corner cutting: locked runs (dungeon walls, fully
            // projected ruin walls) keep their exact corners.
            for _ in 0..params.chaikin_iters {
                plain = chaikin_locked(&plain);
            }
            let flat: Vec<Point> = plain.into_iter().map(|(p, _)| p).collect();
            let mut loops_out = split_bowties(decimate(flat, 0.8));
            for lp in loops_out.iter_mut() {
                for p in lp.iter_mut() {
                    *p = quantize_pt(*p);
                }
            }
            loops_out
        })
        .collect();
    (out, walls)
}

/// Signed-area magnitude of a closed polygon (shoelace).
fn polygon_area(lp: &[Point]) -> f64 {
    let n = lp.len();
    let mut a = 0.0;
    for i in 0..n {
        let (x1, y1) = lp[i];
        let (x2, y2) = lp[(i + 1) % n];
        a += x1 * y2 - x2 * y1;
    }
    (a / 2.0).abs()
}

/// Enforce simple loops: wherever the boundary crosses itself (a "bowtie" —
/// two boundary segments intersecting, e.g. an exact dungeon wall pulled in
/// until it meets the opposite side of a thin neck), **split** the loop into
/// two sub-loops at the crossing rather than discarding a side. A pinch where
/// two real floor regions meet at a point yields two real loops — both kept —
/// so no floor is ever amputated (the old cut-the-shorter-lobe rule deleted
/// whole rooms when the surviving side merely had more vertices). Only lobes
/// below `MIN_AREA` are dropped: those are genuine smoothing slivers (a
/// near-zero-width fold), not floor. Every returned loop is simple.
fn split_bowties(lp: Vec<Point>) -> Vec<Vec<Point>> {
    // A real chamber or corridor is hundreds of px²; a spurious fold left by
    // smoothing is a thin needle far below one cell.
    const MIN_AREA: f64 = 4.0;
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut stack = vec![lp];
    // Each split strictly shrinks both halves, so this terminates; the budget
    // only guards against pathological floating-point near-coincidences.
    let mut budget = 512;
    while let Some(cur) = stack.pop() {
        if cur.len() < 3 {
            continue;
        }
        budget -= 1;
        if budget < 0 {
            out.push(cur);
            continue;
        }
        if let Some((i, j, p)) = first_crossing(&cur) {
            // Segment i..i+1 crosses segment j..j+1 at p (i < j, non-adjacent).
            // Loop A: p, cur[i+1..=j].  Loop B: p, cur[j+1..], cur[..=i].
            let mut a: Vec<Point> = Vec::with_capacity(j - i + 1);
            a.push(p);
            a.extend_from_slice(&cur[i + 1..=j]);
            let mut b: Vec<Point> = Vec::with_capacity(cur.len() - (j - i) + 1);
            b.push(p);
            b.extend_from_slice(&cur[j + 1..]);
            b.extend_from_slice(&cur[..=i]);
            stack.push(a);
            stack.push(b);
        } else {
            out.push(cur);
        }
    }
    out.retain(|l| l.len() >= 3 && polygon_area(l) > MIN_AREA);
    out
}

/// First pair of non-adjacent segments that intersect, with the crossing
/// point, using a coarse spatial hash to stay near-linear.
fn first_crossing(lp: &[Point]) -> Option<(usize, usize, Point)> {
    let n = lp.len();
    if n < 4 {
        return None;
    }
    const CELL: f64 = 16.0;
    let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let key = |x: f64, y: f64| ((x / CELL).floor() as i64, (y / CELL).floor() as i64);
    for i in 0..n {
        let (a, b) = (lp[i], lp[(i + 1) % n]);
        let (k0, k1) = (
            key(a.0.min(b.0), a.1.min(b.1)),
            key(a.0.max(b.0), a.1.max(b.1)),
        );
        for kx in k0.0..=k1.0 {
            for ky in k0.1..=k1.1 {
                buckets.entry((kx, ky)).or_default().push(i);
            }
        }
    }
    let mut best: Option<(usize, usize, Point)> = None;
    for seg in buckets.values() {
        for (si, &i) in seg.iter().enumerate() {
            for &j in &seg[si + 1..] {
                let (i, j) = (i.min(j), i.max(j));
                if j == i + 1 || (i == 0 && j == n - 1) {
                    continue;
                }
                if let Some(p) = seg_intersection(lp[i], lp[(i + 1) % n], lp[j], lp[(j + 1) % n])
                    && best.is_none_or(|(bi, bj, _)| (i, j) < (bi, bj))
                {
                    best = Some((i, j, p));
                }
            }
        }
    }
    best
}

fn seg_intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let r = (b.0 - a.0, b.1 - a.1);
    let s = (d.0 - c.0, d.1 - c.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = ((c.0 - a.0) * s.1 - (c.1 - a.1) * s.0) / denom;
    let u = ((c.0 - a.0) * r.1 - (c.1 - a.1) * r.0) / denom;
    if t > 1e-9 && t < 1.0 - 1e-9 && u > 1e-9 && u < 1.0 - 1e-9 {
        Some((a.0 + r.0 * t, a.1 + r.1 * t))
    } else {
        None
    }
}

/// Drop points closer than `min_d` to the previously kept point; the loops
/// come out of Chaikin far denser than any renderer needs.
fn decimate(lp: Vec<Point>, min_d: f64) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(lp.len() / 2);
    for p in lp {
        let far_enough = out
            .last()
            .is_none_or(|&(lx, ly)| (p.0 - lx).hypot(p.1 - ly) >= min_d);
        if far_enough {
            out.push(p);
        }
    }
    out
}

/// Trace all boundary loops of the floor set. Each loop is a sequence of
/// `(cell, corner_index)` pairs in consistent winding (floor on a fixed
/// side), traced with a cell-following walk so pinch corners are handled
/// unambiguously.
pub(crate) fn trace_loops(floor: &HashSet<Hex>) -> Vec<Vec<(Hex, usize)>> {
    // All directed boundary edges, in deterministic order.
    let mut cells: Vec<Hex> = floor.iter().copied().collect();
    cells.sort_unstable();
    let mut edges: Vec<(Hex, usize)> = Vec::new();
    for &c in &cells {
        for (k, &d) in D.iter().enumerate() {
            if !floor.contains(&c.plus(d)) {
                edges.push((c, k));
            }
        }
    }

    let mut visited: HashSet<(Hex, usize)> = HashSet::new();
    let mut loops = Vec::new();
    for &start in &edges {
        if visited.contains(&start) {
            continue;
        }
        let mut lp = Vec::new();
        let mut cur = start;
        loop {
            visited.insert(cur);
            let (c, k) = cur;
            lp.push((c, k));
            // Advance past corner k+1: stay in this cell if the next side is
            // also wall, otherwise step into the floor cell ahead.
            let ahead = c.plus(D[(k + 1) % 6]);
            cur = if floor.contains(&ahead) {
                (ahead, (k + 5) % 6)
            } else {
                (c, (k + 1) % 6)
            };
            if cur == start {
                break;
            }
        }
        loops.push(lp);
    }
    loops
}

/// Corner `i` of `cell` at angle `60i - 30` degrees.
fn corner_point(cell: Hex, i: usize, size: f64) -> Point {
    crate::grid::hex_corner(cell.center(size), i, size)
}

impl Hex {
    fn plus(self, d: Hex) -> Hex {
        Hex::new(self.q + d.q, self.r + d.r)
    }
}

/// (position, narrow-cell centre, ruin shape, dungeon-wall flag).
type TaggedPoint = (Point, Option<Point>, Option<RuinShape>, bool);

/// Shorter way round between two wall parameters on a closed perimeter of length `per`.
fn cyc_dist(a: f64, b: f64, per: f64) -> f64 {
    let d = (a - b).rem_euclid(per);
    d.min(per - d)
}

/// Shape-tile tracing (D1): replace every maximal run of dungeon-owned
/// vertices with a resampling of the room's exact wall between the run's
/// endpoints. Projecting vertices one-by-one kept the wall hostage to the
/// ragged cell raster — an axis-aligned rectangle cannot tile cleanly on
/// staggered hex rows, and cells excluded as contested eroded organic. The
/// splice *discards* the raster vertices instead: edges between corners come
/// out straight and corners exact by construction. Spliced vertices keep the
/// dungeon flag and shape tag, so every later pass holds them locked and the
/// hard projection is an identity.
///
/// Each emitted run is also pushed onto `walls` (quantized like the final
/// outline; a closed run repeats its first point) for the renderer's thick
/// dungeon-wall band. The locked run survives the rest of the pipeline
/// verbatim up to `decimate`/`quantize`, so the captured polyline coincides
/// with the rendered outline to well under a pixel.
fn splice_dungeon_runs(
    pts: &mut Vec<TaggedPoint>,
    jambs: &[Jamb],
    s: f64,
    walls: &mut Vec<WallRun>,
) {
    // A vertex is splicable when it belongs to a dungeon room *and* carries a
    // room shape (one with a perimeter — rooms, not halls); `perimeter()` is
    // the single source of truth for that.
    let splicable = |t: &TaggedPoint| t.3 && t.2.is_some_and(|sh| sh.perimeter().is_some());
    if pts.len() < 3 || !pts.iter().any(splicable) {
        return;
    }
    // A run endpoint lands wherever the raster happened to leave the wall;
    // snap it to the mouth's jamb (opening centre ± half-opening along the
    // wall) so the gap cut into the wall is exactly the doorway the bar
    // spans — closing gaps that sprawl wider and relieving pinches narrower.
    // `side` picks WHICH jamb edge (+1 → `tw + half`, -1 → `tw - half`): an
    // endpoint must snap to the edge that OPENS its gap — the run's end to
    // the edge where the gap begins in walk direction, the run's start to
    // the edge where it ends. Snapping to the nearest of both edges could
    // pull both endpoints onto the SAME edge (e.g. an opening clamped
    // against a corner), sealing the doorway shut.
    let snap_range = 2.2 * crate::grid::SQRT3 / 2.0 * s;
    let snap = |shape: &RuinShape, t: f64, side: f64| -> f64 {
        let per = shape.perimeter().unwrap_or(0.0);
        let cyc = |a: f64, b: f64| cyc_dist(a, b, per);
        let mut best = (snap_range, t);
        for jamb in jambs.iter().filter(|j| &j.shape == shape) {
            let tw = shape.wall_param(jamb.center);
            let j = (tw + side * jamb.half).rem_euclid(per);
            // A run endpoint that lands *inside* the opening (the raster left it
            // in the gap — e.g. an opening spanning a whole short wall) must
            // retract to the jamb edge however far it is, else the run walks the
            // long way round and doubles a wall stub back across the gap.
            let d = if cyc(t, tw) < jamb.half - 1e-6 {
                -1.0
            } else {
                cyc(t, j)
            };
            if d < best.0 {
                best = (d.max(0.0), j);
            }
        }
        best.1
    };
    // Rotate a non-dungeon vertex to index 0 so runs never wrap. A loop that
    // is wall end to end (a room bounded only by open gaps) becomes the full
    // perimeter.
    match (0..pts.len()).find(|&i| !splicable(&pts[i])) {
        Some(start) => pts.rotate_left(start),
        None => {
            let shape = pts[0].2.unwrap();
            let t0 = shape.wall_param(shape.project(pts[0].0));
            let walk = wall_walk(&shape, t0, 1.0, shape.perimeter().unwrap_or(0.0), s, true);
            let mut run: WallRun = walk.iter().map(|&p| (quantize_pt(p), shape)).collect();
            if let Some(&first) = run.first() {
                run.push(first);
            }
            if run.len() > 2 {
                walls.push(run);
            }
            *pts = walk
                .into_iter()
                .map(|p| (p, None, Some(shape), true))
                .collect();
            return;
        }
    }
    let n = pts.len();
    // Pass 1 — resolve every run to a wall walk, without emitting it. Holding the runs
    // back is what lets `square_seams` fix a junction: squaring it moves the END of one
    // run and the START of the next, so neither can be walked until both are known.
    let mut items: Vec<Item> = Vec::with_capacity(n + 16);
    let mut i = 0;
    while i < n {
        if !splicable(&pts[i]) {
            items.push(Item::Vertex(pts[i]));
            i += 1;
            continue;
        }
        let shape = pts[i].2.unwrap();
        let mut j = i;
        while j + 1 < n && splicable(&pts[j + 1]) && pts[j + 1].2 == Some(shape) {
            j += 1;
        }
        // Run i..=j becomes the exact wall between the projected (and jamb-snapped) run
        // ends, following whichever way around the run itself goes (its middle vertex
        // disambiguates; trivially short runs take the short way).
        let per = shape.perimeter().unwrap_or(0.0);
        let (raw_a, raw_b) = (
            shape.wall_param(shape.project(pts[i].0)),
            shape.wall_param(shape.project(pts[j].0)),
        );
        let fwd_raw = (raw_b - raw_a).rem_euclid(per);
        let forward = if j - i <= 1 {
            fwd_raw <= per - fwd_raw
        } else {
            let tm = shape.wall_param(shape.project(pts[i + (j - i) / 2].0));
            (tm - raw_a).rem_euclid(per) <= fwd_raw + 1e-9
        };
        // A run endpoint jamb-snaps only where the run actually meets a GAP (a
        // non-splicable stretch — a doorway or organic passage). At a fused
        // seam the band continues straight into the next shape's run, and that
        // junction is mid-wall, not a doorway edge: snapping it can grab a
        // nearby door's FAR jamb and inflate the walk to nearly the whole
        // perimeter (measured: a one-vertex run beside a door walked 313 of a
        // 334 perimeter, drawing the room's far wall across open fused floor).
        let d_sign = if forward { 1.0 } else { -1.0 };
        let gap_before = !splicable(&pts[(i + n - 1) % n]);
        let gap_after = !splicable(&pts[(j + 1) % n]);
        // The run's start follows a gap and its end precedes one, so with walk
        // direction `d` the start snaps to a jamb's `tw + d·half` edge and the end
        // to `tw - d·half`.
        let ta = if gap_before {
            snap(&shape, raw_a, d_sign)
        } else {
            raw_a
        };
        let tb = if gap_after {
            snap(&shape, raw_b, -d_sign)
        } else {
            raw_b
        };
        items.push(Item::Run(Run {
            shape,
            per,
            raw_a,
            raw_b,
            ta,
            tb,
            dir: d_sign,
            fwd_raw,
        }));
        i = j + 1;
    }

    // Pass 2 — square each fused seam onto the two borders' true crossing.
    square_seams(&mut items, s);

    // Pass 3 — walk and emit. Seam-adjacent runs (consecutive runs of different
    // shapes with no gap between — where two fused rooms' walls meet) accumulate
    // into one band, flushed only at a real gap (a non-splicable vertex) or the
    // loop's end. A fused compound then renders as one continuous wall, not two
    // capsules notched at the seam. Non-fused rooms keep a rock gap, so every run
    // is gap-bounded and flushes alone.
    let mut out: Vec<TaggedPoint> = Vec::with_capacity(n + 16);
    let mut current: WallRun = Vec::new();
    // Emit the accumulated band (a polyline needs ≥2 points) and reset.
    let mut flush = |current: &mut WallRun| {
        let band = std::mem::take(current);
        if band.len() > 1 {
            walls.push(band);
        }
    };
    for item in items {
        match item {
            Item::Vertex(t) => {
                flush(&mut current);
                out.push(t);
            }
            Item::Run(r) => {
                let walk = wall_walk(&r.shape, r.start(), r.dir, r.len(), s, false);
                // Append this run's wall to the accumulating band, tagging each vertex
                // with the shape it projects onto (so the renderer offsets correctly
                // across a seam). The outline `out` keeps the raw dedup'd points.
                current.extend(walk.iter().map(|&p| (quantize_pt(p), r.shape)));
                for p in walk {
                    if out.last().map(|&(q, _, _, _)| q) != Some(p) {
                        out.push((p, None, Some(r.shape), true));
                    }
                }
            }
        }
    }
    flush(&mut current);
    *pts = out;
}

/// One entry of a spliced loop: either a vertex left organic, or a dungeon run resolved
/// but not yet walked.
enum Item {
    Vertex(TaggedPoint),
    Run(Run),
}

/// A maximal stretch of one room shape's boundary, resolved to a wall walk.
///
/// Kept as data rather than walked on the spot so that [`square_seams`] can move a
/// junction's two endpoints — the end of one run and the start of the next — before
/// either is turned into points.
struct Run {
    shape: RuinShape,
    per: f64,
    /// Where the raster actually left the wall, before any snapping. The length guard is
    /// calibrated on these, so it stays a bound on how far snapping moved things.
    raw_a: f64,
    raw_b: f64,
    /// Post-snap endpoints (jamb snapping at a gap, seam squaring at a fused junction).
    ta: f64,
    tb: f64,
    /// Walk direction: `+1` with increasing wall parameter, `-1` against.
    dir: f64,
    /// Raw forward span, used only to tell a legitimately empty run from a folded one.
    fwd_raw: f64,
}

impl Run {
    fn forward(&self) -> bool {
        self.dir > 0.0
    }

    /// Walk length from `a` to `b` in this run's direction.
    fn span(&self, a: f64, b: f64) -> f64 {
        if self.forward() {
            (b - a).rem_euclid(self.per)
        } else {
            (a - b).rem_euclid(self.per)
        }
    }

    /// Whether the snapped endpoints must be abandoned.
    ///
    /// This guards an **arc ambiguity, not an overshoot**. `wall_walk` takes a start, a
    /// direction and a length along a *closed* perimeter, so two endpoint parameters
    /// describe two arcs — the short way round and the long way. Direction comes from the
    /// raster run's middle vertex, and nothing in that tells which arc is the wall. Move an
    /// endpoint *past* the other and the span wraps: the walk then draws the room's entire
    /// far wall across open floor (measured: a one-vertex run beside a door walked 313 of a
    /// 334 perimeter).
    ///
    /// What moves an endpoint is **jamb snapping** — a doorway's `±half` edge, reconciled
    /// in pixel space against a parameter the raster produced independently. Phase 3a
    /// deletes that, and with it this guard: endpoints taken from tile vertexes are already
    /// on the border and there is nothing to reconcile.
    ///
    /// Seam squaring cannot trip it. [`SEAM_REACH`] caps the move at arc `m`, so the new
    /// length is at most `len_raw + m`, which is exactly the bound below (measured over
    /// four configurations: 14–46 folds each, **none** of them at a seam). Removing the cap
    /// is what inflates the bound until a wrapped walk satisfies it — the failure mode of
    /// the first attempt at phase 2a, which drew walls 30px inside their own rooms.
    fn folded(&self) -> bool {
        let cyc = |a: f64, b: f64| cyc_dist(a, b, self.per);
        let expected =
            self.span(self.raw_a, self.raw_b) + cyc(self.raw_a, self.ta) + cyc(self.raw_b, self.tb);
        let len = self.span(self.ta, self.tb);
        len > expected + 1e-6 || (len < 1e-6 && self.fwd_raw > 1e-6)
    }

    fn start(&self) -> f64 {
        if self.folded() { self.raw_a } else { self.ta }
    }

    fn len(&self) -> f64 {
        if self.folded() {
            self.span(self.raw_a, self.raw_b)
        } else {
            self.span(self.ta, self.tb)
        }
    }
}

/// How far a seam may move a run endpoint, in cells — applied both in the plane and along
/// each border. A cell and a half: the raster junction and the border crossing both sit at
/// the seam, so a larger move means the wrong crossing was picked (measured median: 6–8px,
/// i.e. about half this). The cap is load-bearing, not cosmetic — see [`Run::folded`].
const SEAM_REACH: f64 = 1.5;

/// Square a fused compound's seam corner onto the two borders' **true crossing**.
///
/// Where two fused rooms' runs meet, each run ends wherever the cell raster left its own
/// wall. Those two points differ, so the band jumps straight between them — a chord that
/// cuts inside whichever room it crosses (measured: every wall segment reaching more than
/// 2px into a room in the `fused` configurations is one of these chords or the step
/// immediately after it, and all of them are circle↔circle). The two borders genuinely
/// meet at a point, so the corner can be exact instead: end one run and start the next at
/// the crossing they share. Same geometry as [`compound_wall`], applied to the one corner
/// rather than by rebuilding the whole compound — which is what keeps every opening,
/// doorway and jamb snap on its existing path.
///
/// Declines, leaving the chord, unless all of:
///
/// - the two borders cross in exactly **two** distinct points (so the overlap is one span
///   — halls and hex cells report none at all and fall out here);
/// - the crossing is **local** to the chord it replaces, within [`SEAM_REACH`] both in the
///   plane and along each border. Without that cap a junction can grab the seam's *far*
///   corner, and the walk wraps the long way round the room; the cap is also what keeps
///   [`Run::folded`]'s bound honest.
fn square_seams(items: &mut [Item], s: f64) {
    let reach = SEAM_REACH * s;
    for k in 1..items.len() {
        let (before, after) = items.split_at_mut(k);
        let (Some(Item::Run(a)), Some(Item::Run(b))) = (before.last_mut(), after.first_mut())
        else {
            continue;
        };
        // Exactly two distinct crossings. A rect corner lying on the other's edge is
        // reported by both incident edges, so dedupe before counting.
        let mut xs: Vec<Point> = Vec::new();
        for p in a.shape.border_crossings(&b.shape) {
            if !xs.iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < 1e-6) {
                xs.push(p);
            }
        }
        if xs.len() != 2 {
            continue;
        }
        // The chord this replaces: a's raw end to b's raw start. Neither is jamb-snapped
        // — a seam junction has no gap on either side — so the raw params are the ones
        // actually emitted today.
        let (pa, pb) = (a.shape.wall_point(a.raw_b), b.shape.wall_point(b.raw_a));
        let mid = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
        let Some(&x) = xs
            .iter()
            .min_by(|u, v| {
                let d = |p: &Point| (p.0 - mid.0).hypot(p.1 - mid.1);
                d(u).total_cmp(&d(v))
            })
            .filter(|p| (p.0 - mid.0).hypot(p.1 - mid.1) <= reach)
        else {
            continue;
        };
        let (ta, tb) = (a.shape.wall_param(x), b.shape.wall_param(x));
        if cyc_dist(a.raw_b, ta, a.per) > reach || cyc_dist(b.raw_a, tb, b.per) > reach {
            continue;
        }
        a.tb = ta;
        b.ta = tb;
    }
}

/// Walk a room shape's wall from parameter `ta`, `len` far in direction
/// `dir`, emitting the start point, every feature in between (the shape's
/// corners, plus circle arc samples about every half-cell), and the end
/// point. With `closed` the walk covers the whole perimeter and skips the
/// duplicate endpoint.
fn wall_walk(shape: &RuinShape, ta: f64, dir: f64, len: f64, s: f64, closed: bool) -> Vec<Point> {
    let per = shape.perimeter().unwrap_or(0.0);
    let mut out = vec![shape.wall_point(ta)];
    // Corner seams that fall within the walked span...
    let mut marks: Vec<f64> = shape
        .wall_corners()
        .into_iter()
        .map(|c| (dir * (c - ta)).rem_euclid(per))
        .filter(|&off| off > 1e-6 && off < len - 1e-6)
        .collect();
    // ...plus even arc samples for a smooth circle.
    if matches!(shape, RuinShape::Circle { .. }) {
        let k = (len / (0.5 * s)).ceil().max(1.0) as usize;
        marks.extend((1..k).map(|i| len * i as f64 / k as f64));
    }
    marks.sort_by(f64::total_cmp);
    for off in marks {
        out.push(shape.wall_point((ta + dir * off).rem_euclid(per)));
    }
    if !closed && len > 1e-6 {
        out.push(shape.wall_point((ta + dir * len).rem_euclid(per)));
    }
    out
}

/// The wall of a fused pair, built by **clipping each border against the other**: the arc of
/// `a` that lies outside `b`, then the arc of `b` that lies outside `a`. The two meet at the
/// borders' crossings, so the seam corner is exact — no chord between two projected run ends,
/// which is what chamfers it (`plans/tile-first-render.md` phase 2a).
///
/// Returned closed (first point repeated) and tagged per vertex with the shape that vertex
/// came from, so the renderer offsets each stretch inward on its own geometry across the seam.
///
/// `None` — leaving the caller on its existing path — when the pair is not this construction's
/// business rather than when it fails:
///
/// - either shape has no closed border (a hall, whose `perimeter()` is `None`);
/// - the borders do not cross in exactly **two** distinct points. Two convex borders can meet
///   in more than two (two rects crossing like a `+` meet in eight, and their union is a
///   twelve-sided cross, not two arcs). Every fused pair measured crosses in exactly one span,
///   so this is a guard against the unmeasured case, not a fallback for a common one;
/// - one border lies entirely inside the other, so there is no outside arc to walk.
///
/// Deliberately **no** nearest-crossing choice, no walk direction inferred from a middle
/// vertex, and no length guard: each arc is picked by asking whether its own midpoint is
/// outside the other shape, which has one answer. The first attempt at phase 2a chose the
/// crossing nearest a projected endpoint instead and drew room walls up to 30px inside their
/// own floor.
pub fn compound_wall(a: RuinShape, b: RuinShape, s: f64) -> Option<WallRun> {
    let (per_a, per_b) = (a.perimeter()?, b.perimeter()?);
    // Exactly two distinct crossings. A rect corner sitting on the other's edge is reported
    // by both edges incident to it, so dedupe before counting.
    let mut xs: Vec<Point> = Vec::new();
    for p in a.border_crossings(&b) {
        if !xs.iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < 1e-6) {
            xs.push(p);
        }
    }
    if xs.len() != 2 {
        return None;
    }
    // The arc between the two crossings that lies OUTSIDE `other`, as (start, length).
    // Both candidates are tried rather than one being derived, because which of the two
    // is the outside arc depends on where the shapes sit, not on crossing order.
    let outside_arc = |sh: RuinShape, per: f64, other: RuinShape| -> Option<(f64, f64)> {
        let (t0, t1) = (sh.wall_param(xs[0]), sh.wall_param(xs[1]));
        [
            (t0, (t1 - t0).rem_euclid(per)),
            (t1, (t0 - t1).rem_euclid(per)),
        ]
        .into_iter()
        .find(|&(from, len)| {
            len > 1e-9 && !other.contains(sh.wall_point((from + len / 2.0).rem_euclid(per)))
        })
    };
    let (a_from, a_len) = outside_arc(a, per_a, b)?;
    let (b_from, b_len) = outside_arc(b, per_b, a)?;

    let walk_a = wall_walk(&a, a_from, 1.0, a_len, s, false);
    let end = *walk_a.last()?;
    // `b`'s arc joins the same two crossings, but its stored start may be either of them.
    // Walk it from whichever end `a` finished at, so the two runs meet rather than jump.
    let from_b_start = (b.wall_point(b_from).0 - end.0).hypot(b.wall_point(b_from).1 - end.1);
    let other_end = (b_from + b_len).rem_euclid(per_b);
    let from_b_end = (b.wall_point(other_end).0 - end.0).hypot(b.wall_point(other_end).1 - end.1);
    let walk_b = if from_b_start <= from_b_end {
        wall_walk(&b, b_from, 1.0, b_len, s, false)
    } else {
        wall_walk(&b, other_end, -1.0, b_len, s, false)
    };

    let mut out: WallRun = walk_a.into_iter().map(|p| (quantize_pt(p), a)).collect();
    // Drop `b`'s first point: it is the crossing `a` already ended on.
    out.extend(walk_b.into_iter().skip(1).map(|p| (quantize_pt(p), b)));
    let first = out.first()?.0;
    if out.last().map(|&(p, _)| p) != Some(first) {
        out.push((first, b));
    }
    (out.len() > 3).then_some(out)
}

fn subdivide_tagged(pts: &[TaggedPoint]) -> Vec<TaggedPoint> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let (p, tag, ruin, dungeon) = pts[i];
        let (q, _, _, _) = pts[(i + 1) % pts.len()];
        out.push((p, tag, ruin, dungeon));
        out.push((((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), tag, ruin, dungeon));
    }
    out
}

fn subdivide_locked(pts: &[(Point, bool)]) -> Vec<(Point, bool)> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let (p, pl) = pts[i];
        let (q, ql) = pts[(i + 1) % pts.len()];
        out.push((p, pl));
        out.push((((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), pl && ql));
    }
    out
}

fn smooth(pts: &mut [TaggedPoint], t: f64) {
    let n = pts.len();
    let orig: Vec<Point> = pts.iter().map(|&(p, _, _, _)| p).collect();
    for i in 0..n {
        // Dungeon walls stay on the exact hex boundary.
        if pts[i].3 {
            continue;
        }
        let prev = orig[(i + n - 1) % n];
        let next = orig[(i + 1) % n];
        let mid = ((prev.0 + next.0) / 2.0, (prev.1 + next.1) / 2.0);
        let p = &mut pts[i].0;
        p.0 += (mid.0 - p.0) * t;
        p.1 += (mid.1 - p.1) * t;
    }
}

/// Jitter each unlocked vertex along its local wall normal. Normal-only
/// displacement cannot reorder vertices along the curve, so the jitter
/// passes can no longer fold the loop into micro-bowties the way isotropic
/// jitter could (`remove_bowties` remains as the unconditional guarantee
/// for everything else, e.g. thin necks crossing globally).
fn jitter_unlocked<R: Rng>(pts: &mut [(Point, bool)], mag: f64, rng: &mut R) {
    let n = pts.len();
    // Normals come from a pre-pass snapshot so every vertex sees the same
    // geometry regardless of processing order.
    let orig: Vec<Point> = pts.iter().map(|&(p, _)| p).collect();
    for i in 0..n {
        // Draw for every vertex so the RNG stream doesn't depend on how
        // many vertices happen to be locked.
        let a = rng.random_range(-mag..=mag);
        if pts[i].1 {
            continue;
        }
        let prev = orig[(i + n - 1) % n];
        let next = orig[(i + 1) % n];
        let (tx, ty) = (next.0 - prev.0, next.1 - prev.1);
        let len = tx.hypot(ty);
        if len < 1e-9 {
            continue;
        }
        pts[i].0.0 += ty / len * a;
        pts[i].0.1 -= tx / len * a;
    }
}

/// One round of Chaikin corner cutting on a closed polyline, honouring locks:
/// an edge whose BOTH endpoints are locked is kept verbatim (its first
/// endpoint is emitted unchanged), so locked runs — dungeon walls, fully
/// projected ruin walls — keep their exact corners while everything else
/// rounds as before. Transition edges into organic wall cut normally.
fn chaikin_locked(pts: &[(Point, bool)]) -> Vec<(Point, bool)> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let (p, pl) = pts[i];
        let (q, ql) = pts[(i + 1) % pts.len()];
        if pl && ql {
            out.push((p, true));
        } else {
            out.push(((0.75 * p.0 + 0.25 * q.0, 0.75 * p.1 + 0.25 * q.1), false));
            out.push(((0.25 * p.0 + 0.75 * q.0, 0.25 * p.1 + 0.75 * q.1), false));
        }
    }
    out
}
