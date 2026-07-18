//! Boundary tracing and smoothing: converts the cell-based cave floor into
//! organic closed curves, erasing all traces of the hex grid.
//!
//! Pipeline (after watabou, via Boris the Brave's analysis): trace boundary
//! loops -> subdivide edges -> Laplacian smoothing (bumpiness) -> pull
//! tunnel vertices toward cell centres (narrowing) -> random vertex offsets
//! (irregularity) -> two rounds of subdivide + finer jitter (roughness) ->
//! Chaikin corner cutting.

use crate::grid::Hex;
use crate::growth::Areas;
use crate::ruins::RuinShape;
use crate::topology::Topology;
use rand::Rng;
use std::collections::{HashMap, HashSet};

pub type Point = (f64, f64);

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
/// `ruins::ruin_cell_map`).
pub fn build_outline<R: Rng>(
    areas: &Areas,
    topology: &Topology,
    ruin_cells: &HashMap<Hex, RuinShape>,
    params: &OutlineParams,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let (floor, narrow) = floor_and_narrow(areas, topology);
    smooth_loops(trace_loops(&floor), &narrow, ruin_cells, params, rng)
}

/// Run any cell-set boundary through the full smoothing pipeline. Vertices
/// owned by ruin cells are projected onto their geometric shape and locked
/// against all jitter, so those wall sections stay crisp.
pub(crate) fn smooth_loops<R: Rng>(
    loops: Vec<Vec<(Hex, usize)>>,
    narrow: &HashSet<Hex>,
    ruin_cells: &HashMap<Hex, RuinShape>,
    params: &OutlineParams,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let size = params.hex_size;
    loops
        .into_iter()
        .map(|lp| {
            // Tagged points: position, the owning cell's centre if that cell
            // is a narrow tunnel, and its ruin shape if it has one.
            let mut pts: Vec<(Point, Option<Point>, Option<RuinShape>)> = lp
                .iter()
                .map(|&(cell, corner)| {
                    let tag = narrow.contains(&cell).then(|| cell.center(size));
                    let ruin = ruin_cells.get(&cell).copied();
                    (corner_point(cell, corner, size), tag, ruin)
                })
                .collect();

            pts = subdivide_tagged(&pts);
            for _ in 0..params.smooth_passes {
                smooth(&mut pts, params.bumpiness);
            }
            for (p, tag, _) in pts.iter_mut() {
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
            for (i, &(_, _, ruin)) in pts.iter().enumerate() {
                if ruin.is_none() {
                    dist[i] = 0;
                }
            }
            if dist.iter().any(|&d| d == 0) {
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
            let mut locked = vec![false; n];
            for i in 0..n {
                if let Some(shape) = pts[i].2 {
                    let w = if dist[i] == u32::MAX {
                        1.0
                    } else {
                        (dist[i] as f64 / RAMP).min(1.0)
                    };
                    let p = pts[i].0;
                    let proj = shape.project(p);
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
                .map(|((p, _, _), l)| (p, l))
                .collect();
            jitter_unlocked(&mut plain, params.irregularity * size, rng);
            for round in 0..2 {
                plain = subdivide_locked(&plain);
                let mag = params.roughness * size / (round + 1) as f64;
                jitter_unlocked(&mut plain, mag, rng);
            }
            let mut flat: Vec<Point> = plain.into_iter().map(|(p, _)| p).collect();
            for _ in 0..params.chaikin_iters {
                flat = chaikin(&flat);
            }
            decimate(flat, 0.8)
        })
        .collect()
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
        for k in 0..6 {
            if !floor.contains(&c.plus(D[k])) {
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

type TaggedPoint = (Point, Option<Point>, Option<RuinShape>);

fn subdivide_tagged(pts: &[TaggedPoint]) -> Vec<TaggedPoint> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let (p, tag, ruin) = pts[i];
        let (q, _, _) = pts[(i + 1) % pts.len()];
        out.push((p, tag, ruin));
        out.push((((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), tag, ruin));
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
    let orig: Vec<Point> = pts.iter().map(|&(p, _, _)| p).collect();
    for i in 0..n {
        let prev = orig[(i + n - 1) % n];
        let next = orig[(i + 1) % n];
        let mid = ((prev.0 + next.0) / 2.0, (prev.1 + next.1) / 2.0);
        let p = &mut pts[i].0;
        p.0 += (mid.0 - p.0) * t;
        p.1 += (mid.1 - p.1) * t;
    }
}

fn jitter_unlocked<R: Rng>(pts: &mut [(Point, bool)], mag: f64, rng: &mut R) {
    for (p, locked) in pts.iter_mut() {
        // Draw for every vertex so the RNG stream doesn't depend on how many
        // vertices happen to be locked.
        let dx = rng.random_range(-mag..=mag);
        let dy = rng.random_range(-mag..=mag);
        if !*locked {
            p.0 += dx;
            p.1 += dy;
        }
    }
}

/// One round of Chaikin corner cutting on a closed polyline.
fn chaikin(pts: &[Point]) -> Vec<Point> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let p = pts[i];
        let q = pts[(i + 1) % pts.len()];
        out.push((0.75 * p.0 + 0.25 * q.0, 0.75 * p.1 + 0.25 * q.1));
        out.push((0.25 * p.0 + 0.75 * q.0, 0.25 * p.1 + 0.75 * q.1));
    }
    out
}
