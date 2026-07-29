//! Phase 1b/1c invariant: with `tile_bounded_shapes`, every room tile is bounded by its own
//! area's shape. This is the property the tile-first render is being built on top of — see
//! `plans/tile-first-render.md` — so it is asserted rather than merely printed.
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

use maps_core::ruins::RuinShape;
use maps_core::tags::Tags;
use maps_core::{CaveMap, GenOptions, generate_with};

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
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Some(Tags::parse(tags).unwrap()),
                    ruins_level: Some(1.0),
                    dungeon_level: Some(1.0),
                    fuse_level: Some(1.0),
                    tile_bounded_shapes: Some(true),
                    ..GenOptions::default()
                },
            );
            let bad = unbounded(&m, s);
            assert!(
                bad.is_empty(),
                "seed {seed} [{tags}]: {} tile(s) outside their own area's shape: {bad:?}",
                bad.len()
            );
        }
    }
}
