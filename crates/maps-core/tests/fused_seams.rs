//! Every fused pair's seam must be walled.
//!
//! Two areas fuse by closing the one-cell rock gap growth otherwise keeps, so their floor runs
//! continuously across the seam. Something has to enclose it, and which mechanism depends on
//! whether the two borders overlap — the split `fuse::shapes_overlap` already makes:
//!
//! - **borders overlap** → the pair is open once the internal barrier is cropped, and the
//!   compound's outer wall is the two borders clipped against each other
//!   ([`outline::compound_wall`](maps_core::outline::compound_wall));
//! - **borders do not** → a connector spans the gap. A rect's border sits at its extreme cell
//!   *centres*, so two fused rects are genuinely apart: 6px where the fusion is vertical (the
//!   `s/2` shoulder inset either side of an 18px row pitch) up to 20.8px where it is horizontal
//!   (a full column pitch). Measured over 120 seeds: 118 of 120 such pairs are spanned.
//!
//! A third mechanism covers the narrowest case, and it is easy to miss: a one-tile seam with the
//! borders 6px apart claims no corridor floor and gets no `Trapezoid`. It does not need one —
//! `fuse::corridor_floor` notes that "only the wide corridors ask: the narrow hex-aligned angle
//! neck already runs along cells that are floor" — and the seam is walled by the raw hex boundary
//! those cells carry as `RuinShape::HexCell`. Counting only connectors makes those two pairs look
//! unwalled when they are not, so this checks every shape that can enclose floor.
//!
//! This asserts the seam is covered by one of the three. It is the invariant `sweep`'s `fo` cannot
//! see: `fo` counts *corridor* floor left outside every wall, and a seam between two rooms is
//! ordinary room floor, so it would pass unnoticed.
//!
//! `SEEDS` widens the range (default 40).

use maps_core::grid::Hex;
use maps_core::ruins::RuinShape;
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

#[test]
fn fused_seams_are_walled() {
    let s = GenOptions::default().outline.hex_size;
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut unexpected = Vec::new();
    for seed in 1..=seeds {
        let m = generate_with(
            seed,
            &GenOptions {
                tags: Some(Tags::parse("large,ruins,dungeon,fused").unwrap()),
                ruins_level: Some(1.0),
                dungeon_level: Some(1.0),
                fuse_level: Some(1.0),
                tile_bounded_shapes: Some(true),
                ..GenOptions::default()
            },
        );
        // Every shape that can enclose floor: the rooms themselves and everything the render
        // drew as wall. The room borders are currently never the *sole* cover — the test still
        // passes without them — but the invariant is "something encloses the seam", not "a
        // connector does", so all three mechanisms are enumerated rather than the two that
        // happen to be load-bearing today.
        let mut cover: Vec<RuinShape> = (0..m.areas.count())
            .filter_map(|i| m.areas.shape(i))
            .collect();
        cover.extend(m.dungeon_walls.iter().flatten().map(|&(_, sh)| sh));
        for a in 0..m.areas.count() {
            let Some(sa) = m.areas.shape(a) else { continue };
            for b in (a + 1)..m.areas.count() {
                let Some(sb) = m.areas.shape(b) else { continue };
                let (av, bv): (Vec<Hex>, Vec<Hex>) = (
                    m.areas.room_cells(a).collect(),
                    m.areas.room_cells(b).collect(),
                );
                // Adjacent room tiles mean the rock gap was closed, i.e. the pair is fused.
                let seam: Vec<(Hex, Hex)> = av
                    .iter()
                    .flat_map(|x| bv.iter().map(move |y| (*x, *y)))
                    .filter(|(x, y)| x.distance(*y) == 1)
                    .collect();
                if seam.is_empty() {
                    continue;
                }
                // The midpoint of the closest tile pair is the deepest point of the seam: if
                // anything is left unenclosed, it is there.
                let mid = seam
                    .iter()
                    .map(|(x, y)| {
                        let (p, q) = (x.center(s), y.center(s));
                        ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0)
                    })
                    .min_by(|u, v| {
                        (sa.wall_dist(*u) + sb.wall_dist(*u))
                            .total_cmp(&(sa.wall_dist(*v) + sb.wall_dist(*v)))
                    })
                    .unwrap();
                if !cover.iter().any(|sh| sh.contains(mid)) {
                    unexpected.push((seed, a + 1, b + 1, mid));
                }
            }
        }
    }
    assert!(
        unexpected.is_empty(),
        "fused seam(s) left unwalled — no room border, connector or hex neck encloses the seam \
         midpoint: {unexpected:?}"
    );
}
