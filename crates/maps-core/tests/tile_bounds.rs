//! The contract between growth's tiles and the shapes derived from them. These are the
//! properties the tile-first render is built on (`plans/tile-first-render.md`), so they are
//! asserted rather than merely printed.
//!
//! 1. **A shape bounds its own tiles** (phase 1b/1c, needs `tile_bounded_shapes`).
//! 2. **Every circle's tile set is a complete flower** — growth refuses a ring it cannot fill.
//! 3. **No wall crosses a room it is not fused to.** This is a *consequence* of 2, not an
//!    independent rule: a complete flower's border reaches only the tiles immediately
//!    adjacent to its own, and growth keeps a one-cell rock gap between areas it did not
//!    fuse, so a non-partner's tiles are always two cells away and out of reach. Truncated
//!    rings were the sole source of violations (every one of them, measured), which is why
//!    2 is what makes 3 true.
//!
//! The test is per shape kind, because the two kinds sit differently relative to their tiles:
//!
//! - a **circle** contains its tiles, so a tile with any vertex outside is a failure;
//! - a **rect**'s border runs *inside* its tiles — sides down the outer column's apothem edges,
//!   top and bottom joining the shoulder vertices, while a pointy-top tile reaches a further
//!   `s/2` above and below — so overhang is by design, and only a tile lying **wholly** outside
//!   the rect is a failure.
//!
//! Room cells, not floor cells: corridor floor (`join`) sits outside the room's own border by
//! design, walled by the connector instead, and `derive_shape` likewise never sees it.
//!
//! `SEEDS` widens the seed range (default 24). Cheap — this generates but never renders.

use maps_core::grid::Hex;
use maps_core::ruins::RuinShape;
use maps_core::tags::Tags;
use maps_core::{CaveMap, GenOptions, generate_with};
use std::collections::HashSet;

/// The generation options every test here shares.
fn opts(tags: &str, tile_bounded: bool) -> GenOptions {
    GenOptions {
        tags: Some(Tags::parse(tags).unwrap()),
        ruins_level: Some(1.0),
        dungeon_level: Some(1.0),
        fuse_level: Some(1.0),
        tile_bounded_shapes: Some(tile_bounded),
        ..GenOptions::default()
    }
}

/// Tiles their own area's shape fails to bound, as `(area index, cell)`.
fn unbounded(m: &CaveMap, s: f64) -> Vec<(usize, (i32, i32))> {
    let mut out = Vec::new();
    for i in 0..m.areas.count() {
        let Some(sh) = m.areas.shape(i) else { continue };
        for h in m.areas.room_cells(i) {
            let c = h.center(s);
            let vs = h.corners(s);
            let bad = match sh {
                RuinShape::Rect { .. } => !vs.iter().any(|v| sh.contains(*v)) && !sh.contains(c),
                RuinShape::Circle { .. } => vs.iter().any(|v| !sh.contains(*v)),
                // Halls and hex cells are not fitted to a tile set, so they have no such claim.
                _ => false,
            };
            if bad {
                out.push((i + 1, (h.q, h.r)));
            }
        }
    }
    out
}

#[test]
fn shapes_bound_their_own_tiles() {
    let s = GenOptions::default().outline.hex_size;
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    for tags in ["large,ruins,dungeon,separate", "large,ruins,dungeon,fused"] {
        for seed in 1..=seeds {
            let m = generate_with(seed, &opts(tags, true));
            let bad = unbounded(&m, s);
            assert!(
                bad.is_empty(),
                "seed {seed} [{tags}]: {} tile(s) outside their own area's shape: {bad:?}",
                bad.len()
            );
        }
    }
}

/// Is this tile set a complete flower — every cell within distance `k` of one centre,
/// and nothing missing from that disk?
fn is_flower(cells: &[Hex]) -> bool {
    let set: HashSet<Hex> = cells.iter().copied().collect();
    cells.iter().any(|&c| {
        let k = cells.iter().map(|&o| c.distance(o)).max().unwrap_or(0);
        cells.len() == (3 * k * k + 3 * k + 1) as usize
            && (-k..=k).all(|dq| {
                (-k..=k).all(|dr| {
                    let h = Hex {
                        q: c.q + dq,
                        r: c.r + dr,
                    };
                    c.distance(h) > k || set.contains(&h)
                })
            })
    })
}

/// Point in a convex polygon.
fn in_poly(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let n = poly.len();
    let mut sign = 0i32;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let cr = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        let s = if cr > 1e-9 {
            1
        } else if cr < -1e-9 {
            -1
        } else {
            0
        };
        if s != 0 {
            if sign == 0 {
                sign = s
            } else if sign != s {
                return false;
            }
        }
    }
    true
}

/// Closest approach, in cells, between two areas' room tiles.
fn area_gap(m: &CaveMap, a: usize, b: usize) -> i32 {
    let bs: Vec<Hex> = m.areas.room_cells(b).collect();
    m.areas
        .room_cells(a)
        .flat_map(|x| bs.iter().map(move |y| x.distance(*y)))
        .min()
        .unwrap_or(i32::MAX)
}

/// Growth may only claim a ring it can fill, so every circle ends up a complete flower.
/// A ring truncated by the curved map edge would keep the complete ring's radius while
/// missing its outer arc, and the wall would bulge into the rock beyond — see
/// `Shape::candidates`, which applies the same completeness rule the rect strip already had.
#[test]
fn circles_are_complete_flowers() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    for tb in [false, true] {
        for seed in 1..=seeds {
            let m = generate_with(seed, &opts("large,ruins,dungeon,fused", tb));
            for i in 0..m.areas.count() {
                if !matches!(m.areas.shape(i), Some(RuinShape::Circle { .. })) {
                    continue;
                }
                let cells: Vec<Hex> = m.areas.room_cells(i).collect();
                assert!(
                    is_flower(&cells),
                    "seed {seed} tile_bounded={tb}: area {} is a circle over {} tiles that are \
                     not a complete flower: {cells:?}",
                    i + 1,
                    cells.len()
                );
            }
        }
    }
}

/// A shaped room's wall must never cross the drawn floor of a room it is not fused to.
/// Fused partners are excluded: closing the rock gap is deliberate there, and the render
/// crops the overlapping spans to open the passage.
#[test]
fn walls_stay_out_of_unfused_rooms() {
    let s = GenOptions::default().outline.hex_size;
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    for seed in 1..=seeds {
        let m = generate_with(seed, &opts("large,ruins,dungeon,fused", true));
        let n = m.areas.count();
        for a in 0..n {
            let Some(sa) = m.areas.shape(a) else { continue };
            let Some(per) = sa.perimeter() else { continue };
            for b in 0..n {
                if a == b {
                    continue;
                }
                let Some(sb) = m.areas.shape(b) else { continue };
                if area_gap(&m, a, b) <= 1 {
                    continue; // fused: the render crops these
                }
                // Drawn floor is tiles AND shape: a rect's overhanging tile area is clamped
                // back to the rect interior by the outline splice and never drawn.
                let polys: Vec<Vec<(f64, f64)>> = m
                    .areas
                    .room_cells(b)
                    .map(|h| h.corners(s).to_vec())
                    .collect();
                const N: usize = 1024;
                let hit = (0..N).find(|&k| {
                    let p = sa.wall_point(per * k as f64 / N as f64);
                    sb.contains(p) && polys.iter().any(|q| in_poly(p, q))
                });
                assert!(
                    hit.is_none(),
                    "seed {seed}: area {}'s wall crosses the floor of area {}, which it is \
                     not fused to (rock gap {} cells)",
                    a + 1,
                    b + 1,
                    area_gap(&m, a, b)
                );
            }
        }
    }
}
