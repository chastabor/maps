//! Cosmetic dressing: rubble stones scattered in chambers and Dyson-style
//! hatching along the outside of the cave walls.

use crate::grid::Hex;
use crate::outline::Point;
use rand::Rng;
use rand::seq::SliceRandom;
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

/// True if the wall at this sample belongs to a ruin area: step half a cell
/// toward the floor side and look the cell up.
fn is_ruin_wall(p: Point, nrm: (f64, f64), ruin_cells: &HashSet<Hex>, s: f64) -> bool {
    !ruin_cells.is_empty()
        && ruin_cells.contains(&Hex::at((p.0 - nrm.0 * 0.6 * s, p.1 - nrm.1 * 0.6 * s), s))
}

/// Wall hatching as cone units, the counterpart of the forest's border
/// trees: fans are scattered along the wall the way canopies are scattered
/// along the tree line. Per-unit randomness is the axis angle and how deep
/// inside the cave the fan starts — the renderer's floor mask clips anything
/// on the floor (just as the clearing covers the trees' inner halves), so a
/// fan that starts deep shows only its longest strokes.
///
/// Wall sections owned by ruin areas get *faded stipple dots* instead of
/// fans: circles in a band outside the wall, returned as
/// `(centre, radius, opacity)` — larger and darker at the line, fading out.
pub fn hatching<R: Rng>(
    loops: &[Vec<Point>],
    ruin_cells: &HashSet<Hex>,
    hex_size: f64,
    rng: &mut R,
) -> (Vec<HatchFan>, Vec<(Point, f64, f64)>) {
    let mut out = Vec::new();
    let mut dots = Vec::new();
    for lp in loops {
        for (p, dir) in resample(lp, 8.0) {
            let nrm = (dir.1, -dir.0);
            if is_ruin_wall(p, nrm, ruin_cells, hex_size) {
                // Stipple covering this ~8px stretch of ruin wall: dense,
                // large and dark against the line, fading with distance.
                for _ in 0..rng.random_range(15..23) {
                    let along = rng.random_range(-4.0..4.0);
                    let u: f64 = rng.random();
                    let d = 0.4 + u * u * 9.5;
                    let t = (d / 10.0).min(1.0);
                    let r = (0.95 - 0.55 * t) * rng.random_range(0.7..1.2);
                    let alpha = 0.85 - 0.6 * t;
                    dots.push((
                        (
                            p.0 + dir.0 * along + nrm.0 * d,
                            p.1 + dir.1 * along + nrm.1 * d,
                        ),
                        r,
                        alpha,
                    ));
                }
                continue;
            }
            if rng.random_bool(0.12) {
                continue;
            }
            // The trace walks with floor on a fixed side, so this normal
            // always points into the wall (outward on outer loops, into
            // pillars on holes).
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
    // Random stacking: shuffle so which fan hides which is seed-decided,
    // not a uniform cascade in walk direction.
    out.shuffle(rng);
    (out, dots)
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
///
/// Wall sections owned by ruin areas get *masonry tiles* instead of trees:
/// a single course of small tangent-aligned stone blocks just outside the
/// line, returned separately as quads.
pub fn trees<R: Rng>(
    loops: &[Vec<Point>],
    ruin_cells: &HashSet<Hex>,
    hex_size: f64,
    rng: &mut R,
) -> (Vec<(Vec<Point>, usize)>, Vec<Vec<Point>>) {
    let mut out = Vec::new();
    let mut tiles = Vec::new();
    for lp in loops {
        // Masonry course on ruin wall: sampled denser than the blocks are
        // long, so consecutive blocks overlap and hide parts of one another
        // (draw order does the occlusion; the course is shuffled afterwards
        // so the layering direction is random, not a uniform cascade).
        let samples = resample(lp, 3.2);
        let m = samples.len();
        if m >= 3 {
            // Turn angle at each sample: sharp local maxima are corners and
            // get a mitered L-block wrapping both faces.
            let turn: Vec<f64> = (0..m)
                .map(|i| {
                    let a = samples[(i + m - 1) % m].1;
                    let b = samples[(i + 1) % m].1;
                    (a.0 * b.0 + a.1 * b.1).clamp(-1.0, 1.0).acos()
                })
                .collect();
            for i in 0..m {
                let (p, dir) = samples[i];
                let n = (dir.1, -dir.0);
                if !is_ruin_wall(p, n, ruin_cells, hex_size) {
                    continue;
                }
                let is_corner = turn[i] > 0.6
                    && turn[i] >= turn[(i + m - 1) % m]
                    && turn[i] >= turn[(i + 1) % m];
                if is_corner {
                    let t_in = samples[(i + m - 1) % m].1;
                    let t_out = samples[(i + 1) % m].1;
                    tiles.push(corner_tile(p, t_in, t_out, rng));
                } else {
                    tiles.push(tile(p, dir, n, rng));
                }
            }
        }
        // Band 0: canopies tight against the clearing edge, dense enough to
        // overlap into a continuous band.
        for (p, dir) in resample(lp, 5.5) {
            let n = (dir.1, -dir.0);
            if is_ruin_wall(p, n, ruin_cells, hex_size) {
                continue;
            }
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
            let n = (dir.1, -dir.0);
            if is_ruin_wall(p, n, ruin_cells, hex_size) || rng.random_bool(0.2) {
                continue;
            }
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
            let n = (dir.1, -dir.0);
            if is_ruin_wall(p, n, ruin_cells, hex_size) || rng.random_bool(0.15) {
                continue;
            }
            let r = rng.random_range(5.5..11.5);
            let off = rng.random_range(10.0..19.0);
            let c = (
                p.0 + n.0 * off + rng.random_range(-2.5..2.5),
                p.1 + n.1 * off + rng.random_range(-2.5..2.5),
            );
            out.push((canopy(c, r, rng), 2));
        }
    }
    // Random stacking: shuffle so which canopy/block overlaps which is
    // seed-decided, instead of a uniform cascade in walk direction. The
    // canopy depth bands still draw deepest-first (the renderer groups by
    // band); the shuffle randomises overlap within each band.
    out.shuffle(rng);
    tiles.shuffle(rng);
    (out, tiles)
}

/// One masonry block: a tangent-aligned quad recessed into the wall line
/// (its inner edge tucks under the border stroke), with slight size and
/// corner jitter. Blocks are longer than their sampling interval, so the
/// course overlaps.
fn tile<R: Rng>(p: Point, dir: (f64, f64), n: (f64, f64), rng: &mut R) -> Vec<Point> {
    let half_len = rng.random_range(2.4..3.6);
    let depth = rng.random_range(4.0..5.6);
    let inner = rng.random_range(-1.2..-0.6);
    let corners = [
        (-half_len, inner),
        (half_len, inner),
        (half_len, inner + depth),
        (-half_len, inner + depth),
    ];
    corners
        .iter()
        .map(|&(t, d)| {
            (
                p.0 + dir.0 * t + n.0 * d + rng.random_range(-0.25..0.25),
                p.1 + dir.1 * t + n.1 * d + rng.random_range(-0.25..0.25),
            )
        })
        .collect()
}

/// A corner stone: a mitered block bent around the corner so both faces get
/// an arm — the "L" shape of dressed quoins in the reference style.
fn corner_tile<R: Rng>(
    p: Point,
    t_in: (f64, f64),
    t_out: (f64, f64),
    rng: &mut R,
) -> Vec<Point> {
    let n1 = (t_in.1, -t_in.0);
    let n2 = (t_out.1, -t_out.0);
    let mut bis = (n1.0 + n2.0, n1.1 + n2.1);
    let len = bis.0.hypot(bis.1);
    if len < 0.3 {
        // Near-reversal: no sensible miter, fall back to a straight block.
        return tile(p, t_in, n1, rng);
    }
    bis = (bis.0 / len, bis.1 / len);
    let miter = (1.0 / (bis.0 * n1.0 + bis.1 * n1.1)).clamp(1.0, 2.5);

    let inner = rng.random_range(-1.2..-0.6);
    let depth = rng.random_range(4.0..5.6);
    let arm = rng.random_range(2.8..4.2);
    let ci = (p.0 + bis.0 * inner * miter, p.1 + bis.1 * inner * miter);
    let co = (
        p.0 + bis.0 * (inner + depth) * miter,
        p.1 + bis.1 * (inner + depth) * miter,
    );
    let pts = [
        (p.0 - t_in.0 * arm + n1.0 * inner, p.1 - t_in.1 * arm + n1.1 * inner),
        (
            p.0 - t_in.0 * arm + n1.0 * (inner + depth),
            p.1 - t_in.1 * arm + n1.1 * (inner + depth),
        ),
        co,
        (
            p.0 + t_out.0 * arm + n2.0 * (inner + depth),
            p.1 + t_out.1 * arm + n2.1 * (inner + depth),
        ),
        (p.0 + t_out.0 * arm + n2.0 * inner, p.1 + t_out.1 * arm + n2.1 * inner),
        ci,
    ];
    pts.iter()
        .map(|&(x, y)| {
            (
                x + rng.random_range(-0.25..0.25),
                y + rng.random_range(-0.25..0.25),
            )
        })
        .collect()
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
