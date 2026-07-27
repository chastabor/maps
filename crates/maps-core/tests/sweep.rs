//! Whole-map sweep: generate every tag configuration over a range of seeds and
//! measure the geometric invariants the fuse-connector work is held to, plus a
//! content digest per configuration.
//!
//! Ignored by default — it renders many hundreds of maps twice each. Run it with:
//!
//! ```text
//! cargo test -p maps-core --test sweep --release -- --ignored --nocapture
//! ```
//!
//! Env knobs: `SEEDS` widens the seed range (default 120); `RUINS`, `DUNGEON` and
//! `FUSE` force the corresponding generation levels, which the tag defaults leave
//! sparse — the connector work is much easier to see at high levels:
//!
//! ```text
//! SEEDS=200 RUINS=1 DUNGEON=1 FUSE=1 cargo test -p maps-core --test sweep --release -- --ignored --nocapture
//! ```
//!
//! **What is asserted** (these fail the test):
//! - `w6` — no dungeon wall-band segment runs more than 6px inside a room's interior.
//!   The hard bar. Holds for every configuration at both level settings.
//! - `fo` — a configuration with no connectors (`conn == 0`) must leak no corridor
//!   floor, since it has no corridors to leak. Structural, so it is enforced.
//!
//! **What is only printed** — compare against the table in
//! `plans/fuse-case-taxonomy.md` → CURRENT STATE, which records the current values:
//! - `digest` — content hash of `svg` + `debug_svg` over the whole range. Not asserted
//!   here (that is `golden.rs`'s job for its own configuration); its use is
//!   *containment*: run the sweep before and after a change, and a configuration that
//!   should not have moved must keep its digest. That is how growth-time fusion was
//!   shown to touch only fused maps.
//! - `si` — maps with a self-intersecting floor outline. A probe with a substantial
//!   pre-existing false-positive rate (see the taxonomy's NEXT STEP section — the set
//!   is identical with fusion off, so it is a dungeon-splice problem). Churn in this
//!   number is triaged by eyeballing the seeds that changed, not by asserting it.
//! - `conn` — wall runs carrying a connector. More is better; the count is by run, so
//!   a connector contributes more than one.
//! - `fo` for a configuration that *does* have connectors. Known small defect: a few
//!   corridor cells survive `release_unused_claims` outside every wall — at 200 seeds,
//!   4 maps at tag defaults (seeds 41, 127, 138, 152) and 1 dense (seed 24). Tracked in
//!   the taxonomy backlog; the bar is zero, so this is printed with its seeds rather
//!   than asserted at its current value.

use maps_core::render::{debug_svg, svg};
use maps_core::ruins::RuinShape;
use maps_core::tags::Tags;
use maps_core::{CaveMap, GenOptions, generate_with};

const CONFIGS: [&str; 5] = [
    "large,organic,separate",
    "medium,coral,wet,organic,mosaic",
    "large,ruins,dungeon,separate",
    "large,ruins,dungeon,fused",
    "large,chamber,connected,ruins,dungeon,truchet",
];

/// How far inside a room a wall segment must reach to be reported.
const DEPTH_REPORT: f64 = 6.0;

fn fnv(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Whether segments `a→b` and `c→d` properly cross (shared endpoints do not count).
fn crosses(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let s = [side(a, b, c), side(a, b, d), side(c, d, a), side(c, d, b)].map(f64::signum);
    s[0] != s[1] && s[2] != s[3] && s.iter().all(|&x| x != 0.0)
}

/// Whether a closed loop crosses itself, ignoring adjacent (endpoint-sharing) edges.
fn self_intersects(loop_: &[(f64, f64)]) -> bool {
    let n = loop_.len();
    if n < 4 {
        return false;
    }
    (0..n).any(|i| {
        ((i + 2)..n)
            .filter(|&j| !(i == 0 && j == n - 1))
            .any(|j| crosses(loop_[i], loop_[(i + 1) % n], loop_[j], loop_[(j + 1) % n]))
    })
}

/// Whether `p` is inside a wall geometry. Every [`RuinShape`] variant, so the count
/// does not swing with the mix of shapes a configuration happens to produce.
fn inside(p: (f64, f64), sh: RuinShape) -> bool {
    const APOTHEM: f64 = 0.866_025_403_784_438_6; // √3/2
    match sh {
        RuinShape::Circle { cx, cy, r } => (p.0 - cx).hypot(p.1 - cy) <= r + 1e-6,
        RuinShape::Rect { cx, cy, hw, hh } => {
            (p.0 - cx).abs() <= hw + 1e-6 && (p.1 - cy).abs() <= hh + 1e-6
        }
        RuinShape::StraightHall { ax, ay, bx, by, hw } => {
            let d = (bx - ax, by - ay);
            let l2 = (d.0 * d.0 + d.1 * d.1).max(1e-9);
            let t = (((p.0 - ax) * d.0 + (p.1 - ay) * d.1) / l2).clamp(0.0, 1.0);
            (p.0 - (ax + d.0 * t)).hypot(p.1 - (ay + d.1 * t)) <= hw + 1e-6
        }
        RuinShape::ArcHall { cx, cy, r, hw } => ((p.0 - cx).hypot(p.1 - cy) - r).abs() <= hw + 1e-6,
        // A pointy-top hex contains p iff p is within the apothem of the centre on
        // all three edge normals (0° and ±60°).
        RuinShape::HexCell { cx, cy, s } => {
            let (dx, dy) = (p.0 - cx, p.1 - cy);
            [(1.0, 0.0), (0.5, APOTHEM), (-0.5, APOTHEM)]
                .iter()
                .all(|&(nx, ny)| (dx * nx + dy * ny).abs() <= APOTHEM * s + 1e-6)
        }
    }
}

/// Corridor floor that no wall encloses: fusion-corridor cells whose centre falls
/// inside none of the map's wall geometries.
///
/// Restricted to corridor floor on purpose. A room's own cells sit outside its shape
/// for reasons that predate fusion — the door and exit stubs `topology` adds after the
/// shape is derived — so counting those buries the signal under a large baseline (51
/// cells for `large,ruins,dungeon,separate`, where no fusion happens at all).
fn corridor_floor_outside(m: &CaveMap, s: f64) -> usize {
    let mut shapes: Vec<RuinShape> = m.ruins.iter().flatten().copied().collect();
    shapes.extend(m.dungeon_walls.iter().flatten().map(|&(_, sh)| sh));
    m.areas
        .join()
        .iter()
        .filter(|c| m.areas.owner_of(**c).is_some())
        .filter(|c| {
            let p = c.center(s);
            !shapes.iter().any(|&sh| inside(p, sh))
        })
        .count()
}

/// How far the deepest dungeon wall segment reaches inside a room's interior, in px.
///
/// A wall belongs on a room's boundary; a segment well inside one means the band took
/// a shortcut across open floor. Sampled along each segment rather than at vertices,
/// since the vertices are exactly the points that *are* on a boundary.
fn worst_interior_depth(m: &CaveMap) -> f64 {
    let mut worst = 0.0f64;
    for run in &m.dungeon_walls {
        for pair in run.windows(2) {
            let (p, q) = (pair[0].0, pair[1].0);
            if (p.0 - q.0).hypot(p.1 - q.1) < 1.0 {
                continue;
            }
            for t in 1..8 {
                let f = t as f64 / 8.0;
                let c = (p.0 + (q.0 - p.0) * f, p.1 + (q.1 - p.1) * f);
                for sh in m.ruins.iter().flatten() {
                    let depth = match *sh {
                        RuinShape::Circle { cx, cy, r } => r - (c.0 - cx).hypot(c.1 - cy),
                        RuinShape::Rect { cx, cy, hw, hh } => {
                            (hw - (c.0 - cx).abs()).min(hh - (c.1 - cy).abs())
                        }
                        _ => -1.0,
                    };
                    worst = worst.max(depth);
                }
            }
        }
    }
    worst
}

#[test]
#[ignore = "renders hundreds of maps; run explicitly with --ignored --nocapture"]
fn sweep() {
    let level = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok());
    let seeds = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120u64);
    let hex_size = GenOptions::default().outline.hex_size;

    for tag_str in CONFIGS {
        let tags = Tags::parse(tag_str).unwrap();
        let (mut digest, mut si, mut conn, mut fo) = (0u64, 0, 0, 0usize);
        let (mut deep, mut worst) = (Vec::new(), 0.0f64);
        let mut leaks = Vec::new();

        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Some(tags.clone()),
                    ruins_level: level("RUINS"),
                    dungeon_level: level("DUNGEON"),
                    fuse_level: level("FUSE"),
                    ..GenOptions::default()
                },
            );
            // Rotate by seed so the xor-fold is order-sensitive, and fold both
            // renderers so a change to either shows up.
            digest ^= fnv(&svg(&m)).rotate_left((seed % 63) as u32);
            digest ^= fnv(&debug_svg(&m)).rotate_left((seed % 31) as u32);

            if m.outline.iter().any(|lp| self_intersects(lp)) {
                si += 1;
            }
            let n = corridor_floor_outside(&m, hex_size);
            if n > 0 {
                fo += n;
                leaks.push((seed, n));
            }
            let d = worst_interior_depth(&m);
            worst = worst.max(d);
            if d > DEPTH_REPORT {
                deep.push((seed, d));
            }
            conn += m
                .dungeon_walls
                .iter()
                .filter(|r| {
                    r.iter()
                        .any(|&(_, sh)| matches!(sh, RuinShape::StraightHall { .. }))
                })
                .count();
        }

        println!(
            "{tag_str}\n  digest={digest:016x} si={si} conn={conn} fo={fo} worst={worst:.1}px"
        );
        for (seed, n) in &leaks {
            println!("  corridor floor outside every wall: seed={seed} cells={n}");
        }
        for (seed, d) in &deep {
            println!("  wall {d:.1}px inside a room: seed={seed}");
        }

        assert!(
            deep.is_empty(),
            "{tag_str}: wall band runs >{DEPTH_REPORT}px inside a room on {} seed(s): {deep:?}",
            deep.len()
        );
        // A configuration with no connectors has no corridors, so it has no corridor
        // floor to leak — zero is structural here, unlike for a fusing configuration.
        assert!(
            conn > 0 || leaks.is_empty(),
            "{tag_str}: no connectors, yet corridor floor was left outside every wall: {leaks:?}"
        );
    }
}
