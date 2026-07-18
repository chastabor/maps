//! Water pools. Every floor cell has a terrain elevation (an independent
//! value-noise field); water fills the cells lying below the water level,
//! lowest basins first. Raising the level expands the pools, lowering it
//! makes them recede. Pool boundaries are smoothed with the same pipeline
//! as the cave walls.

use crate::grid::Hex;
use crate::growth::Areas;
use crate::outline::{OutlineParams, Point, floor_and_narrow, smooth_loops, trace_loops};
use crate::tags::{Tags, WaterTag};
use crate::topology::Topology;
use rand::Rng;
use std::collections::HashSet;

/// All water-derived layers, from one elevation field and one water level.
#[derive(Default)]
pub struct Water {
    /// Flooded cells (for decoration placement).
    pub cells: HashSet<Hex>,
    /// Pool outlines at the waterline.
    pub pools: Vec<Vec<Point>>,
    /// Deep-water band: terrain far enough below the level.
    pub deep: Vec<Vec<Point>>,
    /// Mud fringe: damp ground just above the waterline, around the pools.
    pub mud: Vec<Vec<Point>>,
}

/// How far below the water level the deep band starts.
const DEEP_DROP: f64 = 0.15;
/// How far above the water level the mud fringe reaches.
const MUD_RISE: f64 = 0.06;

pub fn build_water<R: Rng>(
    areas: &Areas,
    topology: &Topology,
    params: &OutlineParams,
    tags: &Tags,
    level_override: Option<f64>,
    rng: &mut R,
) -> Water {
    // Draw the noise salt unconditionally so the RNG stream doesn't depend
    // on the water tag or level.
    let salt: u64 = rng.random();
    let level = level_override
        .unwrap_or(match tags.water {
            Some(WaterTag::Dry) => 0.0,
            Some(WaterTag::Wet) => 0.45,
            None => 0.15,
        })
        .clamp(0.0, 1.0);
    if level <= 0.0 {
        return Water::default();
    }

    let (floor, _) = floor_and_narrow(areas, topology);
    let scale = params.hex_size * 3.2;
    let elev = |h: &Hex| {
        let (x, y) = h.center(params.hex_size);
        elevation(x / scale, y / scale, salt)
    };

    // The level is a fill fraction: the waterline threshold is the level-th
    // quantile of this map's elevations, so 0 floods nothing, 0.5 floods the
    // lowest half of the terrain and 1 submerges everything.
    let mut sorted: Vec<f64> = floor.iter().map(&elev).collect();
    sorted.sort_by(f64::total_cmp);
    let idx = (level * sorted.len() as f64) as usize;
    let waterline = if idx >= sorted.len() {
        f64::INFINITY
    } else {
        sorted[idx]
    };

    let mut water: HashSet<Hex> = floor.iter().copied().filter(|h| elev(h) < waterline).collect();
    drop_small_ponds(&mut water, 3);
    if water.is_empty() {
        return Water::default();
    }

    // Mud: the next elevation band up, but only where it borders a real pool
    // (a damp low spot with no pond isn't drawn).
    let mut mud: HashSet<Hex> = floor
        .iter()
        .copied()
        .filter(|h| elev(h) < waterline + MUD_RISE)
        .collect();
    keep_components_touching(&mut mud, &water);

    // Deep water: the band far enough under the waterline, inside the pools.
    let mut deep: HashSet<Hex> = water
        .iter()
        .copied()
        .filter(|h| elev(h) < waterline - DEEP_DROP)
        .collect();
    drop_small_ponds(&mut deep, 2);

    // Water lies flat: far more smoothing and almost no jitter, so pool
    // edges come out glassy compared to the rough rock walls.
    let wparams = OutlineParams {
        bumpiness: 0.65,
        smooth_passes: 5,
        irregularity: 0.03,
        roughness: 0.015,
        narrow_pull: 0.0,
        chaikin_iters: 3,
        ..params.clone()
    };
    let no_ruins = std::collections::HashMap::new();
    let mud_loops = smooth_loops(trace_loops(&mud), &HashSet::new(), &no_ruins, &wparams, rng);
    let pools = smooth_loops(trace_loops(&water), &HashSet::new(), &no_ruins, &wparams, rng);
    let deep_loops = smooth_loops(trace_loops(&deep), &HashSet::new(), &no_ruins, &wparams, rng);

    Water {
        cells: water,
        pools,
        deep: deep_loops,
        mud: mud_loops,
    }
}

/// Keep only the connected components of `set` that contain at least one
/// cell of `anchor`.
fn keep_components_touching(set: &mut HashSet<Hex>, anchor: &HashSet<Hex>) {
    let mut cells: Vec<Hex> = set.iter().copied().collect();
    cells.sort_unstable();
    let mut seen: HashSet<Hex> = HashSet::new();
    for &start in &cells {
        if seen.contains(&start) {
            continue;
        }
        let mut comp = vec![start];
        let mut stack = vec![start];
        seen.insert(start);
        while let Some(c) = stack.pop() {
            for n in c.neighbors() {
                if set.contains(&n) && seen.insert(n) {
                    stack.push(n);
                    comp.push(n);
                }
            }
        }
        if !comp.iter().any(|c| anchor.contains(c)) {
            for c in comp {
                set.remove(&c);
            }
        }
    }
}

/// Remove connected water components smaller than `min` cells.
fn drop_small_ponds(water: &mut HashSet<Hex>, min: usize) {
    let mut cells: Vec<Hex> = water.iter().copied().collect();
    cells.sort_unstable();
    let mut seen: HashSet<Hex> = HashSet::new();
    for &start in &cells {
        if seen.contains(&start) {
            continue;
        }
        let mut comp = vec![start];
        let mut stack = vec![start];
        seen.insert(start);
        while let Some(c) = stack.pop() {
            for n in c.neighbors() {
                if water.contains(&n) && seen.insert(n) {
                    stack.push(n);
                    comp.push(n);
                }
            }
        }
        if comp.len() < min {
            for c in comp {
                water.remove(&c);
            }
        }
    }
}

/// Terrain elevation: two-octave value noise in roughly [0, 1].
fn elevation(x: f64, y: f64, salt: u64) -> f64 {
    (value_noise(x, y, salt) + 0.5 * value_noise(x * 2.0, y * 2.0, salt ^ 0xA5A5_5A5A)) / 1.5
}

fn value_noise(x: f64, y: f64, salt: u64) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = x - ix;
    let fy = y - iy;
    // Smoothstep for C1-continuous interpolation.
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let (ix, iy) = (ix as i64, iy as i64);
    let v00 = lattice(ix, iy, salt);
    let v10 = lattice(ix + 1, iy, salt);
    let v01 = lattice(ix, iy + 1, salt);
    let v11 = lattice(ix + 1, iy + 1, salt);
    let top = v00 + (v10 - v00) * sx;
    let bot = v01 + (v11 - v01) * sx;
    top + (bot - top) * sy
}

/// Deterministic integer-lattice hash in [0, 1).
fn lattice(ix: i64, iy: i64, salt: u64) -> f64 {
    let mut h = salt
        ^ (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}
