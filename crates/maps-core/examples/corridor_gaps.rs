//! What the tile-first ring gap would cut, next to what the current derivation cuts — the
//! phase-3 feasibility measurement (`plans/tile-corridor-render.md`).
//!
//! `corridor::Landing::gaps` is the stretch of a room's border a passage covers, derived from the
//! tiles. `doorway::jambs` is what the outline splices to today, derived from door cells via
//! `porch_chord`. Phase 3b replaces the second with the first, so the question worth answering
//! BEFORE doing that is how far apart they already are, and where.
//!
//! Reports, over a seed range: the distribution of gap widths and porch-chord lengths against the
//! lattice classes (`s`, `√3·s`, `2s` = 12 / 20.78 / 24px at s=12), how many landings come out as
//! more than one gap, how many gaps straddle a rect corner (the case point space cannot express),
//! and the paired difference against the nearest jamb on the same room.
//!
//! ```text
//! SEEDS=200 cargo run -p maps-core --release --example corridor_gaps
//! ```

mod common;

use maps_core::corridor::corridors;
use maps_core::doorway::jambs;
use maps_core::grid::HEX_SIZE as S;
use maps_core::ruins::RuinShape;

const CONFIGS: [&str; 3] = [
    "large,ruins,dungeon,separate",
    "large,ruins,dungeon,fused",
    "large,chamber,connected,ruins,dungeon,truchet",
];

/// A rect's four corner parameters, so a span can be asked whether it turns one.
fn corner_params(sh: &RuinShape) -> Vec<f64> {
    match *sh {
        RuinShape::Rect { hw, hh, .. } => {
            let (w, h) = (2.0 * hw, 2.0 * hh);
            vec![0.0, w, w + h, 2.0 * w + h]
        }
        _ => Vec::new(),
    }
}

/// Which lattice class a length falls in, to 1px.
fn class(len: f64) -> String {
    let names = [("s", S), ("√3s", 3f64.sqrt() * S), ("2s", 2.0 * S)];
    for (n, v) in names {
        if (len - v).abs() < 1.0 {
            return n.to_string();
        }
    }
    format!("{:.0}px", len)
}

fn main() {
    let seeds: u64 = common::env("SEEDS", 200);
    for tags in CONFIGS {
        let (mut n_land, mut n_gaps, mut multi, mut straddle) = (0usize, 0usize, 0usize, 0usize);
        let mut widths: std::collections::BTreeMap<String, usize> = Default::default();
        let mut chords: std::collections::BTreeMap<String, usize> = Default::default();
        // Paired against the jamb the outline uses on the same room today.
        let (mut paired, mut unpaired) = (0usize, 0usize);
        let mut straddle_seeds: Vec<(u64, usize)> = Vec::new();
        let mut deltas: Vec<f64> = Vec::new();
        for seed in 1..=seeds {
            let m = common::generate(seed, tags);
            let js = jambs(&m.mouths, &m.topology, &m.areas, S);
            for cor in corridors(&m.areas, &m.topology.connections, S) {
                for (land, side) in cor.attach.iter().zip([cor.a, cor.b]) {
                    let Some(sh) = m.areas.room_border(side) else {
                        continue;
                    };
                    let Some(per) = sh.perimeter() else { continue };
                    if land.gaps.is_empty() {
                        continue;
                    }
                    n_land += 1;
                    if land.gaps.len() > 1 {
                        multi += 1;
                    }
                    for g in &land.gaps {
                        n_gaps += 1;
                        *widths.entry(class(g.span.len)).or_default() += 1;
                        let cl = (g.chord.0.0 - g.chord.1.0).hypot(g.chord.0.1 - g.chord.1.1);
                        *chords.entry(class(cl)).or_default() += 1;
                        // A gap that turns a rect corner: a corner parameter strictly inside it.
                        if corner_params(&sh).iter().any(|&t| {
                            g.span.contains(t, per)
                                && (t - g.span.from).rem_euclid(per) > 1e-6
                                && (t - g.span.end(per)).rem_euclid(per) > 1e-6
                        }) {
                            straddle += 1;
                            if straddle_seeds.len() < 8 {
                                straddle_seeds.push((seed, side));
                            }
                        }
                        // The jamb on this same shape whose centre is nearest the gap's midpoint.
                        let mid = sh.wall_point((g.span.from + g.span.len / 2.0).rem_euclid(per));
                        match js
                            .iter()
                            .filter(|j| j.shape == sh)
                            .min_by(|x, y| {
                                let d = |c: (f64, f64)| (c.0 - mid.0).hypot(c.1 - mid.1);
                                d(x.center).total_cmp(&d(y.center))
                            })
                            .filter(|j| (j.center.0 - mid.0).hypot(j.center.1 - mid.1) < 3.0 * S)
                        {
                            Some(j) => {
                                paired += 1;
                                deltas.push(g.span.len - 2.0 * j.half);
                            }
                            None => unpaired += 1,
                        }
                    }
                }
            }
        }
        deltas.sort_by(f64::total_cmp);
        let pick = |f: f64| deltas[((deltas.len() as f64 - 1.0) * f) as usize];
        println!("=== {tags} ({seeds} seeds)");
        println!(
            "  landings {n_land}  gaps {n_gaps}  multi-gap landings {multi}  \
             corner-straddling gaps {straddle}"
        );
        println!("  corner-straddling (seed, room): {straddle_seeds:?}");
        println!("  gap width classes:   {widths:?}");
        println!("  porch chord classes: {chords:?}");
        if deltas.is_empty() {
            println!("  vs today's jambs: none paired ({unpaired} gaps had no jamb within 3s)");
        } else {
            println!(
                "  vs today's jambs: {paired} paired, {unpaired} unpaired; gap minus jamb width \
                 p05 {:+.1} med {:+.1} p95 {:+.1} px",
                pick(0.05),
                pick(0.5),
                pick(0.95)
            );
        }
    }
}
