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
use crate::topology::Topology;
use rand::Rng;
use std::collections::HashSet;

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
/// by winding; render with fill-rule="evenodd").
pub fn build_outline<R: Rng>(
    areas: &Areas,
    topology: &Topology,
    params: &OutlineParams,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let (floor, narrow) = floor_and_narrow(areas, topology);
    smooth_loops(trace_loops(&floor), &narrow, params, rng)
}

/// Run any cell-set boundary through the full smoothing pipeline.
pub(crate) fn smooth_loops<R: Rng>(
    loops: Vec<Vec<(Hex, usize)>>,
    narrow: &HashSet<Hex>,
    params: &OutlineParams,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let size = params.hex_size;
    loops
        .into_iter()
        .map(|lp| {
            // Tagged points: position + the owning cell if that cell is a
            // narrow tunnel (corridor, door or exit passage).
            let mut pts: Vec<(Point, Option<Point>)> = lp
                .iter()
                .map(|&(cell, corner)| {
                    let tag = narrow.contains(&cell).then(|| cell.center(size));
                    (corner_point(cell, corner, size), tag)
                })
                .collect();

            pts = subdivide_tagged(&pts);
            for _ in 0..params.smooth_passes {
                smooth(&mut pts, params.bumpiness);
            }
            for (p, tag) in pts.iter_mut() {
                if let Some(c) = tag {
                    p.0 += (c.0 - p.0) * params.narrow_pull;
                    p.1 += (c.1 - p.1) * params.narrow_pull;
                }
            }

            let mut plain: Vec<Point> = pts.into_iter().map(|(p, _)| p).collect();
            jitter(&mut plain, params.irregularity * size, rng);
            for round in 0..2 {
                plain = subdivide(&plain);
                let mag = params.roughness * size / (round + 1) as f64;
                jitter(&mut plain, mag, rng);
            }
            for _ in 0..params.chaikin_iters {
                plain = chaikin(&plain);
            }
            decimate(plain, 0.8)
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

fn subdivide_tagged(pts: &[(Point, Option<Point>)]) -> Vec<(Point, Option<Point>)> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let (p, tag) = pts[i];
        let (q, _) = pts[(i + 1) % pts.len()];
        out.push((p, tag));
        out.push((((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), tag));
    }
    out
}

fn subdivide(pts: &[Point]) -> Vec<Point> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for i in 0..pts.len() {
        let p = pts[i];
        let q = pts[(i + 1) % pts.len()];
        out.push(p);
        out.push(((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0));
    }
    out
}

fn smooth(pts: &mut [(Point, Option<Point>)], t: f64) {
    let n = pts.len();
    let orig: Vec<Point> = pts.iter().map(|&(p, _)| p).collect();
    for i in 0..n {
        let prev = orig[(i + n - 1) % n];
        let next = orig[(i + 1) % n];
        let mid = ((prev.0 + next.0) / 2.0, (prev.1 + next.1) / 2.0);
        let p = &mut pts[i].0;
        p.0 += (mid.0 - p.0) * t;
        p.1 += (mid.1 - p.1) * t;
    }
}

fn jitter<R: Rng>(pts: &mut [Point], mag: f64, rng: &mut R) {
    for p in pts.iter_mut() {
        p.0 += rng.random_range(-mag..=mag);
        p.1 += rng.random_range(-mag..=mag);
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
