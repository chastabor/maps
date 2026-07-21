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

pub type Point = (f64, f64);

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
    for (i, area) in areas.cells.iter().enumerate() {
        for &c in area {
            floor.insert(c);
            if topology.is_corridor[i] {
                narrow.insert(c);
            }
        }
    }
    for d in &topology.doors {
        floor.insert(d.cell);
        narrow.insert(d.cell);
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
pub fn build_outline<R: Rng>(
    areas: &Areas,
    topology: &Topology,
    ruin_cells: &HashMap<Hex, RuinShape>,
    dungeon_cells: &HashMap<Hex, RuinShape>,
    jambs: &[Jamb],
    params: &OutlineParams,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let (floor, narrow) = floor_and_narrow(areas, topology);
    smooth_loops(trace_loops(&floor), &narrow, ruin_cells, dungeon_cells, jambs, params, rng)
}

/// Run any cell-set boundary through the full smoothing pipeline. Vertices
/// owned by ruin cells are projected onto their geometric shape and locked
/// against all jitter, so those wall sections stay crisp; runs owned by
/// dungeon cells are replaced wholesale by the room's exact wall (see
/// `splice_dungeon_runs`) and locked from the start.
pub(crate) fn smooth_loops<R: Rng>(
    loops: Vec<Vec<(Hex, usize)>>,
    narrow: &HashSet<Hex>,
    ruin_cells: &HashMap<Hex, RuinShape>,
    dungeon_cells: &HashMap<Hex, RuinShape>,
    jambs: &[Jamb],
    params: &OutlineParams,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let size = params.hex_size;
    loops
        .into_iter()
        .map(|lp| {
            // Tagged points: position, the owning cell's centre if that cell
            // is a narrow tunnel, its ruin shape if it has one, and whether
            // it belongs to a dungeon room. A dungeon cell's shape comes from
            // `dungeon_cells` (every room cell, even ones `ruin_cell_map`
            // excludes as contested — the splice overrides those).
            let mut pts: Vec<TaggedPoint> = lp
                .iter()
                .map(|&(cell, corner)| {
                    let tag = narrow.contains(&cell).then(|| cell.center(size));
                    let dungeon_shape = dungeon_cells.get(&cell).copied();
                    let ruin = dungeon_shape.or_else(|| ruin_cells.get(&cell).copied());
                    (corner_point(cell, corner, size), tag, ruin, dungeon_shape.is_some())
                })
                .collect();

            // Shape-tile tracing: swap each dungeon run's raster vertices for
            // the exact wall before any smoothing or random pass sees them.
            splice_dungeon_runs(&mut pts, jambs, size);

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
                        RuinShape::Rect { .. } | RuinShape::Circle { .. } => 1.0,
                        RuinShape::StraightHall { .. } | RuinShape::ArcHall { .. } => {
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
            let mut lp = remove_bowties(decimate(flat, 0.8));
            for p in lp.iter_mut() {
                *p = quantize_pt(*p);
            }
            lp
        })
        .collect()
}

/// Enforce simple loops: wherever the boundary crosses itself (a "bowtie" —
/// e.g. two ruin shapes' wall loci intersecting), cut the smaller lobe off
/// at the crossing point. Guarantees the rendered border never loops over
/// itself regardless of what upstream projection did.
fn remove_bowties(mut lp: Vec<Point>) -> Vec<Point> {
    for _ in 0..4 {
        let Some((i, j, p)) = first_crossing(&lp) else {
            return lp;
        };
        let lobe = j - i;
        if lobe <= lp.len() / 2 {
            lp.splice(i + 1..=j, [p]);
        } else {
            let mut kept: Vec<Point> = lp[i + 1..=j].to_vec();
            kept.push(p);
            lp = kept;
        }
        if lp.len() < 3 {
            return lp;
        }
    }
    lp
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
        let (k0, k1) = (key(a.0.min(b.0), a.1.min(b.1)), key(a.0.max(b.0), a.1.max(b.1)));
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
                if let Some(p) =
                    seg_intersection(lp[i], lp[(i + 1) % n], lp[j], lp[(j + 1) % n])
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
    let (cx, cy) = cell.center(size);
    let angle = std::f64::consts::PI / 180.0 * (60.0 * i as f64 - 30.0);
    (cx + size * angle.cos(), cy + size * angle.sin())
}

impl Hex {
    fn plus(self, d: Hex) -> Hex {
        Hex::new(self.q + d.q, self.r + d.r)
    }
}

/// (position, narrow-cell centre, ruin shape, dungeon-wall flag).
type TaggedPoint = (Point, Option<Point>, Option<RuinShape>, bool);

/// Shape-tile tracing (D1): replace every maximal run of dungeon-owned
/// vertices with a resampling of the room's exact wall between the run's
/// endpoints. Projecting vertices one-by-one kept the wall hostage to the
/// ragged cell raster — an axis-aligned rectangle cannot tile cleanly on
/// staggered hex rows, and cells excluded as contested eroded organic. The
/// splice *discards* the raster vertices instead: edges between corners come
/// out straight and corners exact by construction. Spliced vertices keep the
/// dungeon flag and shape tag, so every later pass holds them locked and the
/// hard projection is an identity.
fn splice_dungeon_runs(pts: &mut Vec<TaggedPoint>, jambs: &[Jamb], s: f64) {
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
    let snap_range = 2.2 * crate::grid::SQRT3 / 2.0 * s;
    let snap = |shape: &RuinShape, t: f64| -> f64 {
        let per = shape.perimeter().unwrap_or(0.0);
        let cyc = |a: f64, b: f64| {
            let d = (a - b).rem_euclid(per);
            d.min(per - d)
        };
        let mut best = (snap_range, t);
        for jamb in jambs.iter().filter(|j| &j.shape == shape) {
            let tw = shape.wall_param(jamb.center);
            for j in [(tw - jamb.half).rem_euclid(per), (tw + jamb.half).rem_euclid(per)] {
                let d = cyc(t, j);
                if d < best.0 {
                    best = (d, j);
                }
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
            *pts = wall_walk(&shape, t0, 1.0, shape.perimeter().unwrap_or(0.0), s, true)
                .into_iter()
                .map(|p| (p, None, Some(shape), true))
                .collect();
            return;
        }
    }
    let n = pts.len();
    let mut out: Vec<TaggedPoint> = Vec::with_capacity(n + 16);
    let mut i = 0;
    while i < n {
        if !splicable(&pts[i]) {
            out.push(pts[i]);
            i += 1;
            continue;
        }
        let shape = pts[i].2.unwrap();
        let mut j = i;
        while j + 1 < n && splicable(&pts[j + 1]) && pts[j + 1].2 == Some(shape) {
            j += 1;
        }
        // Replace run i..=j with the exact wall between the projected (and
        // jamb-snapped) run ends, following whichever way around the run
        // itself goes (its middle vertex disambiguates; trivially short runs
        // take the short way).
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
        // A snap that folds the run to nothing (both ends grabbed by one
        // jamb) falls back to the raw endpoints.
        let len_of = |a: f64, b: f64| {
            if forward { (b - a).rem_euclid(per) } else { (a - b).rem_euclid(per) }
        };
        let mut ta = snap(&shape, raw_a);
        let mut len = len_of(ta, snap(&shape, raw_b));
        if len < 1e-6 && fwd_raw > 1e-6 {
            ta = raw_a;
            len = len_of(raw_a, raw_b);
        }
        let dir = if forward { 1.0 } else { -1.0 };
        for p in wall_walk(&shape, ta, dir, len, s, false) {
            if out.last().map(|&(q, _, _, _)| q) != Some(p) {
                out.push((p, None, Some(shape), true));
            }
        }
        i = j + 1;
    }
    *pts = out;
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
