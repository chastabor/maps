//! A fused compound's seam corner is a **point**, not a chord.
//!
//! Where two fused rooms' wall runs meet, each run used to end wherever the cell raster left
//! its own border. The two points differ, so the band jumped straight between them, and that
//! chord cut inside whichever room it crossed — the deepest wall-band intrusions in every
//! fusing configuration were one of those chords or the step right after it. The two borders
//! genuinely meet at a point, so the corner is now taken from
//! [`border_crossings`](maps_core::ruins::RuinShape::border_crossings) instead
//! (`plans/tile-first-render.md` phase 2a).
//!
//! Asserted here as a property of the output rather than of the code path, so it survives
//! phase 3 replacing how the endpoints are computed: **at a junction between two room
//! borders that cross, the two runs share one point, and it lies on both borders.**
//!
//! `SEEDS` widens the seed range (default 40).

use maps_core::ruins::RuinShape;
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

/// Distinct crossings of two borders. A rect corner lying on the other's edge is reported by
/// both incident edges, so dedupe before counting.
fn crossings(a: &RuinShape, b: &RuinShape) -> Vec<(f64, f64)> {
    let mut xs: Vec<(f64, f64)> = Vec::new();
    for p in a.border_crossings(b) {
        if !xs.iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < 1e-6) {
            xs.push(p);
        }
    }
    xs
}

#[test]
fn fused_seam_corners_are_points() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let (mut squared, mut chords, mut near_misses) = (0usize, 0usize, Vec::new());
    for tags in [
        "large,ruins,dungeon,fused",
        "large,chamber,connected,ruins,dungeon,truchet",
    ] {
        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Some(Tags::parse(tags).unwrap()),
                    ruins_level: Some(1.0),
                    dungeon_level: Some(1.0),
                    fuse_level: Some(1.0),
                    ..GenOptions::default()
                },
            );
            for run in &m.dungeon_walls {
                for w in run.windows(2) {
                    let ((p, sa), (q, sb)) = (w[0], w[1]);
                    // Room-to-room seams only. A connector (`Trapezoid`/`StraightHall`) or a
                    // hex neck also meets a room's wall at a junction, but its geometry comes
                    // from `fuse`'s contact tiles and already lands on the border — a
                    // different mechanism, and phase 3b's to change.
                    let room = |sh: &RuinShape| {
                        matches!(sh, RuinShape::Circle { .. } | RuinShape::Rect { .. })
                    };
                    if sa == sb || !room(&sa) || !room(&sb) {
                        continue;
                    }
                    // Classify by the OUTCOME, not by re-deriving whether squaring should
                    // have applied — a test that mirrors the predicate only asserts that the
                    // code equals itself. A junction is either a point or a chord.
                    let gap = (p.0 - q.0).hypot(p.1 - q.1);
                    // Quantized to a tenth of a pixel, and each side reaches the corner
                    // through its own shape's parameterisation, so allow one step.
                    if gap <= 0.15 {
                        squared += 1;
                        // It is the crossing, not merely a shared point: on both borders.
                        assert_eq!(
                            crossings(&sa, &sb).len(),
                            2,
                            "seed {seed} [{tags}]: runs meet at {p:?} but the borders do not \
                             cross in one span — {sa:?} / {sb:?}"
                        );
                        for sh in [sa, sb] {
                            let d = sh.wall_dist(p);
                            assert!(
                                d <= 0.15,
                                "seed {seed} [{tags}]: shared corner {p:?} sits {d:.2}px off \
                                 {sh:?} — it is not the border crossing"
                            );
                        }
                    } else {
                        chords += 1;
                        // Nothing may land in between. A junction part-way to its crossing
                        // would mean an endpoint was moved without the two sides agreeing,
                        // which is the failure the first attempt at phase 2a produced.
                        if gap < 1.0 {
                            near_misses.push((seed, tags, gap));
                        }
                    }
                }
            }
        }
    }
    assert!(
        squared > 0,
        "no fused seam corner was squared — the probe is not reaching the case"
    );
    assert!(
        near_misses.is_empty(),
        "{} junction(s) part-way to their crossing: {near_misses:?}",
        near_misses.len()
    );
    // Not a bar to hold, just a record of reach: a junction stays a chord when the two
    // borders do not cross in one span (rect pairs mostly, whose borders sit inside their
    // tiles) or when the crossing is too far to be this corner's.
    println!("squared {squared} seam corners, {chords} left as chords");
}
