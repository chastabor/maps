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
                    // `through` is defined as "some a-side opposite some b-side".
                    let opp = (0..6).any(|k| ax.toward_a[k] && ax.toward_b[(k + 3) % 6]);
                    assert_eq!(ax.through, opp, "seed {seed}: through flag inconsistent");
                }
                for (land, side) in cor.attach.iter().zip([cor.a, cor.b]) {
                    let Some(sh) = m.areas.shape(side) else {
                        assert!(
                            land.marks.is_empty(),
                            "seed {seed}: landing marks on a shapeless side"
                        );
                        continue;
                    };
                    for mark in &land.marks {
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
