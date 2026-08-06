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

/// Phase 2 (`plans/tile-corridor-render.md`): properties every corridor wall must have.
///
/// Asserted as properties of the OUTPUT, deliberately not by re-deriving the placement rule —
/// a test that mirrors the search would only assert the code equals itself. Two of these are
/// stronger than the search's own acceptance test rather than equal to it:
///
/// - the unclaimed-ground check samples four times as densely as the search does, so a narrow
///   escape between the search's samples fails here;
/// - the inward direction is recovered from the wall PAIR (each wall's normal toward the other)
///   rather than from the frame the search used, so a sign error in that frame cannot hide.
#[test]
fn wall_invariants() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let s = 12.0;
    let (mut walled, mut caps, mut samples) = (0usize, 0usize, 0usize);
    // Collected rather than asserted one at a time, so a run reports the whole set.
    let mut narrow: Vec<(u64, &str, f64)> = Vec::new();
    for tags in [
        "large,ruins,dungeon,separate",
        "large,ruins,dungeon,fused",
        "large,chamber,connected,ruins,dungeon,truchet",
    ] {
        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Tags::parse(tags).ok(),
                    ..GenOptions::default()
                },
            );
            for cor in corridors(&m.areas, &m.topology.connections, s) {
                let w = cor.walls(&m.areas, s);
                let present: Vec<&Vec<(f64, f64)>> = w.iter().filter(|v| v.len() >= 2).collect();
                let claimed = cor.tiles.iter().filter(|&&t| m.areas.is_join(t)).count();

                // The guard: no claimed floor, no walls. `topology` reserves join floor for
                // dungeon-to-dungeon connections only, and walling free cells would put
                // dungeon masonry across a cave passage.
                if claimed == 0 {
                    assert!(
                        present.is_empty(),
                        "seed {seed} [{tags}]: walls on a corridor with no claimed floor"
                    );
                    continue;
                }
                if present.is_empty() {
                    continue;
                }
                walled += 1;

                for side in &present {
                    // Each wall is a single straight segment — no chamfer at either cap, so a
                    // square jamb is available for a door.
                    assert_eq!(
                        side.len(),
                        2,
                        "seed {seed} [{tags}]: wall is not a single segment"
                    );
                    // On a lattice line: the direction is a multiple of 30 degrees, the only
                    // angles a pointy-top lattice offers.
                    let d = (side[1].0 - side[0].0, side[1].1 - side[0].1);
                    let deg = d.1.atan2(d.0).to_degrees().rem_euclid(30.0);
                    let off = deg.min(30.0 - deg);
                    assert!(
                        off < 0.01,
                        "seed {seed} [{tags}]: wall {off:.3} deg off the lattice"
                    );
                    // Caps land ON a room border wherever the abutting side has one. A side
                    // without a border (organic or hall-shaped) takes the tile bound instead,
                    // so only require that SOME end is on a border when either side has one.
                    let borders: Vec<_> = [cor.a, cor.b]
                        .into_iter()
                        .filter_map(|r| m.areas.room_border(r))
                        .collect();
                    if !borders.is_empty() {
                        let on_border =
                            |p: (f64, f64)| borders.iter().any(|sh| sh.wall_dist(p) < 0.5);
                        assert!(
                            on_border(side[0]) || on_border(side[1]),
                            "seed {seed} [{tags}]: neither wall end reaches a border"
                        );
                        caps += 1;
                    }
                }

                if present.len() == 2 {
                    let (a, b) = (present[0], present[1]);
                    // One frame, so the pair is parallel.
                    let ua = {
                        let d = (a[1].0 - a[0].0, a[1].1 - a[0].1);
                        let l = d.0.hypot(d.1).max(1e-9);
                        (d.0 / l, d.1 / l)
                    };
                    let ub = {
                        let d = (b[1].0 - b[0].0, b[1].1 - b[0].1);
                        let l = d.0.hypot(d.1).max(1e-9);
                        (d.0 / l, d.1 / l)
                    };
                    let cross = (ua.0 * ub.1 - ua.1 * ub.0).abs();
                    assert!(
                        cross < 1e-6,
                        "seed {seed} [{tags}]: wall pair not parallel (cross {cross:.2e})"
                    );
                    // A lane wide enough to walk: the two walls bound the passage, so they
                    // must be separated, and by at least one tile's narrow width.
                    let nrm = (-ua.1, ua.0);
                    let lane = ((b[0].0 - a[0].0) * nrm.0 + (b[0].1 - a[0].1) * nrm.1).abs();
                    if lane < s - 0.01 {
                        narrow.push((seed, tags, lane));
                    }
                    // Neither wall crosses unclaimed ground. Inward is taken from the PAIR —
                    // each wall's normal pointing at the other — so this does not depend on
                    // the frame the search used.
                    for (side, other) in [(a, b), (b, a)] {
                        let mid_other = (
                            (other[0].0 + other[1].0) / 2.0,
                            (other[0].1 + other[1].1) / 2.0,
                        );
                        let toward =
                            (mid_other.0 - side[0].0) * nrm.0 + (mid_other.1 - side[0].1) * nrm.1;
                        let sgn = if toward >= 0.0 { 1.0 } else { -1.0 };
                        // 50 samples where the search uses 13: a narrow escape it stepped over
                        // fails here.
                        for q in 0..=50 {
                            let t = q as f64 / 50.0;
                            let p = (
                                side[0].0 + (side[1].0 - side[0].0) * t,
                                side[0].1 + (side[1].1 - side[0].1) * t,
                            );
                            let g = (p.0 + nrm.0 * sgn * 0.6 * s, p.1 + nrm.1 * sgn * 0.6 * s);
                            samples += 1;
                            assert!(
                                m.areas.owner_of(maps_core::grid::Hex::at(g, s)).is_some(),
                                "seed {seed} [{tags}]: wall crosses unclaimed ground at {p:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    // Zero walls would mean the probe never reached the case, not that the rules hold.
    assert!(walled > 100, "only {walled} walled corridors seen");
    assert!(caps > 100, "only {caps} border caps checked");
    assert!(samples > 10_000, "only {samples} ground samples taken");
    // A lane narrower than one tile means the two walls are not both bounding lines of the
    // passage — an apothem-wide lane (10.39px) is the signature of one wall sitting on a tile
    // CENTRE line rather than an edge or shoulder line.
    assert!(
        narrow.is_empty(),
        "{} corridor(s) with a sub-tile lane: {:?}",
        narrow.len(),
        &narrow[..narrow.len().min(12)]
    );
}
