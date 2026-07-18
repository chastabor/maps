//! Cosmetic dressing: rubble stones scattered in chambers and Dyson-style
//! hatching along the outside of the cave walls.

use crate::grid::Hex;
use crate::outline::Point;
use rand::Rng;
use std::collections::HashSet;

/// Small irregular polygons dropped in open (non-tunnel, non-water) cells.
pub fn stones<R: Rng>(
    floor: &HashSet<Hex>,
    narrow: &HashSet<Hex>,
    water: &HashSet<Hex>,
    hex_size: f64,
    rng: &mut R,
) -> Vec<Vec<Point>> {
    let mut open: Vec<Hex> = floor
        .iter()
        .copied()
        .filter(|c| !narrow.contains(c) && !water.contains(c))
        .collect();
    open.sort_unstable();
    if open.is_empty() {
        return Vec::new();
    }

    let count = (open.len() / 22).max(1) + rng.random_range(0..3);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let cell = open[rng.random_range(0..open.len())];
        let (cx, cy) = cell.center(hex_size);
        let cx = cx + rng.random_range(-0.35..0.35) * hex_size;
        let cy = cy + rng.random_range(-0.35..0.35) * hex_size;
        let radius = hex_size * rng.random_range(0.16..0.38);
        let k = rng.random_range(5..=7);
        let mut poly = Vec::with_capacity(k);
        for i in 0..k {
            let angle = i as f64 / k as f64 * std::f64::consts::TAU + rng.random_range(-0.25..0.25);
            let r = radius * rng.random_range(0.7..1.3);
            poly.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
        out.push(poly);
    }
    out
}

/// One wall-hatch cone: five parallel strokes whose lengths grow along the
/// fan's axis, plus the fan's opaque footprint. Fans are objects: the hull
/// is filled with the background colour beneath the strokes, so a fan hides
/// any earlier fan it overlaps — just as overlapping tree canopies hide
/// each other in the forest.
pub struct HatchFan {
    /// Footprint quad from the short end to the wide end, slightly padded.
    pub hull: Vec<Point>,
    pub strokes: Vec<(Point, Point)>,
}

/// Wall hatching as cone units, the counterpart of the forest's border
/// trees: fans are scattered along the wall the way canopies are scattered
/// along the tree line. Per-unit randomness is the axis angle and how deep
/// inside the cave the fan starts — the renderer's floor mask clips anything
/// on the floor (just as the clearing covers the trees' inner halves), so a
/// fan that starts deep shows only its longest strokes.
pub fn hatching<R: Rng>(loops: &[Vec<Point>], rng: &mut R) -> Vec<HatchFan> {
    let mut out = Vec::new();
    for lp in loops {
        for (p, dir) in resample(lp, 8.0) {
            if rng.random_bool(0.12) {
                continue;
            }
            // The trace walks with floor on a fixed side, so this normal
            // always points into the wall (outward on outer loops, into
            // pillars on holes).
            let nrm = (dir.1, -dir.0);
            let swing = rng.random_range(-0.9..0.9f64);
            let (sin, cos) = swing.sin_cos();
            let axis = (nrm.0 * cos - nrm.1 * sin, nrm.0 * sin + nrm.1 * cos);
            let perp = (-axis.1, axis.0);

            // Negative start = the fan begins inside the cave and loses its
            // shortest strokes to the floor mask.
            let start = rng.random_range(-6.0..2.5);
            let step = rng.random_range(2.0..2.8);
            let base_len = rng.random_range(1.8..2.8);
            let grow = rng.random_range(2.2..3.0);
            let mut strokes = Vec::with_capacity(5);
            for k in 0..5 {
                let along = start + k as f64 * step;
                let c = (
                    p.0 + axis.0 * along + rng.random_range(-0.5..0.5),
                    p.1 + axis.1 * along + rng.random_range(-0.5..0.5),
                );
                let half = (base_len + grow * k as f64) / 2.0;
                strokes.push((
                    (c.0 - perp.0 * half, c.1 - perp.1 * half),
                    (c.0 + perp.0 * half, c.1 + perp.1 * half),
                ));
            }
            let hull = fan_hull(&strokes, 1.5);
            out.push(HatchFan { hull, strokes });
        }
    }
    out
}

/// Footprint quad spanning a fan's first (short) and last (wide) strokes,
/// padded outward from the centroid so jittered stroke ends stay covered.
fn fan_hull(strokes: &[(Point, Point)], pad: f64) -> Vec<Point> {
    let (a0, b0) = strokes[0];
    let (a4, b4) = strokes[strokes.len() - 1];
    let corners = [a0, b0, b4, a4];
    let cx = corners.iter().map(|c| c.0).sum::<f64>() / 4.0;
    let cy = corners.iter().map(|c| c.1).sum::<f64>() / 4.0;
    corners
        .iter()
        .map(|&(x, y)| {
            let d = (x - cx).hypot(y - cy).max(1e-9);
            (x + (x - cx) / d * pad, y + (y - cy) / d * pad)
        })
        .collect()
}

/// Tree canopies hugging the tree-line side of every outline loop (forest
/// mode). Each canopy is a spiky star polygon — outer lobe points alternating
/// with a recessed inner ring — in a wide range of sizes. Three rows recede
/// from the clearing; the returned depth band (0 = nearest) drives shading:
/// nearest trees render lightest and the deepest fade toward the dark woods.
pub fn trees<R: Rng>(loops: &[Vec<Point>], rng: &mut R) -> Vec<(Vec<Point>, usize)> {
    let mut out = Vec::new();
    for lp in loops {
        // Band 0: canopies tight against the clearing edge, dense enough to
        // overlap into a continuous band.
        for (p, dir) in resample(lp, 5.5) {
            let n = (dir.1, -dir.0);
            let r = rng.random_range(3.5..8.5);
            let off = r * rng.random_range(0.4..0.7);
            let c = (
                p.0 + n.0 * off + rng.random_range(-1.5..1.5),
                p.1 + n.1 * off + rng.random_range(-1.5..1.5),
            );
            out.push((canopy(c, r, rng), 0));
        }
        // Band 1: bigger canopies deeper into the woods.
        for (p, dir) in resample(lp, 8.0) {
            if rng.random_bool(0.2) {
                continue;
            }
            let n = (dir.1, -dir.0);
            let r = rng.random_range(5.0..10.5);
            let off = rng.random_range(5.0..12.0);
            let c = (
                p.0 + n.0 * off + rng.random_range(-2.0..2.0),
                p.1 + n.1 * off + rng.random_range(-2.0..2.0),
            );
            out.push((canopy(c, r, rng), 1));
        }
        // Band 2: the deep-woods fringe, blending toward the background.
        for (p, dir) in resample(lp, 8.0) {
            if rng.random_bool(0.15) {
                continue;
            }
            let n = (dir.1, -dir.0);
            let r = rng.random_range(5.5..11.5);
            let off = rng.random_range(10.0..19.0);
            let c = (
                p.0 + n.0 * off + rng.random_range(-2.5..2.5),
                p.1 + n.1 * off + rng.random_range(-2.5..2.5),
            );
            out.push((canopy(c, r, rng), 2));
        }
    }
    out
}

/// One canopy: a star polygon whose lobe count grows with its radius, with
/// every second vertex recessed toward the trunk and everything jittered.
fn canopy<R: Rng>(c: Point, r: f64, rng: &mut R) -> Vec<Point> {
    let lobes = (r * 1.3).round().max(5.0) as usize;
    let k = lobes * 2;
    let phase = rng.random_range(0.0..std::f64::consts::TAU);
    let step = std::f64::consts::TAU / k as f64;
    (0..k)
        .map(|i| {
            let angle = phase + i as f64 * step + rng.random_range(-0.3..0.3) * step;
            let rad = if i % 2 == 0 {
                r * rng.random_range(0.85..1.15)
            } else {
                r * rng.random_range(0.5..0.72)
            };
            (c.0 + rad * angle.cos(), c.1 + rad * angle.sin())
        })
        .collect()
}

/// Points every `spacing` units along a closed polyline, each with the unit
/// direction of the edge it sits on.
fn resample(lp: &[Point], spacing: f64) -> Vec<(Point, (f64, f64))> {
    let mut out = Vec::new();
    let mut carry = 0.0;
    for i in 0..lp.len() {
        let (x1, y1) = lp[i];
        let (x2, y2) = lp[(i + 1) % lp.len()];
        let len = (x2 - x1).hypot(y2 - y1);
        if len < 1e-9 {
            continue;
        }
        let dir = ((x2 - x1) / len, (y2 - y1) / len);
        let mut t = spacing - carry;
        while t <= len {
            out.push(((x1 + dir.0 * t, y1 + dir.1 * t), dir));
            t += spacing;
        }
        carry = len - (t - spacing);
    }
    out
}
