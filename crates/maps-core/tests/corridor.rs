//! Invariants of the phase-0 corridor derivation (`plans/tile-corridor-render.md`).
//!
//! The visual acceptance test is the growth-view overlay against
//! `samples/grow-tile-render.png`; what is asserted here is only what must hold for every
//! map: the derivation is total over dungeon connections, axes are consistent with their
//! own flags, and landings actually land on the fitted borders.

use maps_core::corridor::{Mark, corridors};
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

#[test]
fn corridor_invariants() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let s = 12.0;
    let (mut n_cor, mut n_marks) = (0usize, 0usize);
    for tags in ["large,ruins,dungeon,separate", "large,ruins,dungeon,fused"] {
        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Tags::parse(tags).ok(),
                    ..GenOptions::default()
                },
            );
            for cor in corridors(&m.areas, &m.topology.connections, s) {
                n_cor += 1;
                assert!(!cor.tiles.is_empty(), "seed {seed}: corridor with no tiles");
                assert_eq!(cor.tiles.len(), cor.axes.len());
                for ax in &cor.axes {
                    // `toward_*` is a superset of `touch_*` by construction — a side that
                    // faces the room is also a side the passage runs toward.
                    for k in 0..6 {
                        assert!(
                            (!ax.touch_a[k] || ax.toward_a[k])
                                && (!ax.touch_b[k] || ax.toward_b[k]),
                            "seed {seed}: touch without toward"
                        );
                    }
                }
                for (land, side) in cor.attach.iter().zip([cor.a, cor.b]) {
                    let Some(sh) = m.areas.room_border(side) else {
                        assert!(
                            land.is_empty(),
                            "seed {seed}: landing marks on a borderless side"
                        );
                        continue;
                    };
                    for (tile_idx, mark) in land {
                        assert!(*tile_idx < cor.tiles.len());
                        n_marks += 1;
                        let pts = match mark {
                            Mark::Point(p) => vec![*p],
                            Mark::Bar(p, q) => vec![*p, *q],
                        };
                        for p in pts {
                            assert!(
                                sh.wall_dist(p) < 0.5,
                                "seed {seed}: landing mark {p:?} is {:.2}px off its border",
                                sh.wall_dist(p)
                            );
                        }
                    }
                }
            }
        }
    }
    // The derivation must actually be exercised — zero corridors would mean the probe
    // never reached the case, not that the invariants hold.
    assert!(n_cor > 100, "only {n_cor} corridors derived");
    assert!(n_marks > 100, "only {n_marks} landing marks derived");
}

#[test]
fn centerline_invariants() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let s = 12.0;
    let mut n_lines = 0usize;
    for tags in ["large,ruins,dungeon,separate", "large,ruins,dungeon,fused"] {
        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Tags::parse(tags).ok(),
                    ..GenOptions::default()
                },
            );
            for cor in corridors(&m.areas, &m.topology.connections, s) {
                let line = &cor.centerline;
                if line.is_empty() {
                    continue;
                }
                n_lines += 1;
                assert!(line.len() >= 2, "seed {seed}: degenerate centerline");
                // Ends ON the fitted borders (when a side has one).
                for (p, side) in [(line[0], cor.a), (line[line.len() - 1], cor.b)] {
                    if let Some(sh) = m.areas.room_border(side) {
                        assert!(
                            sh.wall_dist(p) < 0.5,
                            "seed {seed}: centerline end {p:?} is {:.2}px off the border",
                            sh.wall_dist(p)
                        );
                    }
                }
                // R4 binds the corridor's INTERIOR: every waypoint between the landings
                // stays within the corridor's own tiles. The two END points are exempt —
                // they sit ON the fitted border (asserted above), and the border may lie a
                // step beyond the last corridor tile, inside the room's own tile: reaching
                // it is the cap. They are still bounded below.
                for p in &line[1..line.len() - 1] {
                    // Interior waypoints are tile centres and shared-edge midpoints, so the
                    // cell lookup lands in one of the corridor's own tiles either way.
                    assert!(
                        cor.tiles.contains(&maps_core::grid::Hex::at(*p, s)),
                        "seed {seed}: centerline point {p:?} escapes the corridor's tiles"
                    );
                }
                for p in [line[0], line[line.len() - 1]] {
                    let near = cor
                        .tiles
                        .iter()
                        .map(|t| {
                            let c = t.center(s);
                            (p.0 - c.0).hypot(p.1 - c.1)
                        })
                        .fold(f64::MAX, f64::min);
                    assert!(
                        near <= 2.0 * s,
                        "seed {seed}: landing {p:?} is {near:.1}px from the corridor"
                    );
                }
                // No teleporting: consecutive waypoints at most two tiles apart.
                for w in line.windows(2) {
                    let d = (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1);
                    assert!(
                        d <= 2.0 * 2.0 * s,
                        "seed {seed}: centerline jump of {d:.1}px"
                    );
                }
            }
        }
    }
    assert!(n_lines > 100, "only {n_lines} centerlines derived");
}
