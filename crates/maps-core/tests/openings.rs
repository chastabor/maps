//! The preconditions phase 2's clipping construction rests on.
//!
//! Clipping means: draw a room's own border, then remove the span between the two points where
//! the thing meeting it crosses. That is only well defined if the crossing set is unambiguous, so
//! these are the two properties worth holding — asserted, not measured once:
//!
//! - **a fused pair's borders overlap in one span or not at all** — never several. Several would
//!   mean a room's wall alternates in and out of its partner and "the span to remove" has no
//!   single answer. Two rects crossing like a `+` would do it: eight crossings, and a union that
//!   is a twelve-sided cross rather than two arcs.
//! - **a connector's wall stretch crosses a room's border at most once.** More would mean one
//!   throat needs several disjoint cuts in the same wall.
//!
//! Both hold with no exceptions at 200 seeds. The printed distribution is also the coverage
//! record phase 2 reads its three cases off — see `plans/tile-first-render.md`.
//!
//! `SEEDS` widens the seed range (default 60).

use maps_core::ruins::RuinShape as R;
use maps_core::tags::Tags;
use maps_core::{CaveMap, GenOptions, generate_with};

type P = (f64, f64);

fn is_room(sh: &R) -> bool {
    matches!(sh, R::Circle { .. } | R::Rect { .. })
}

fn kind(sh: &R) -> &'static str {
    match sh {
        R::Circle { .. } => "circle",
        R::Rect { .. } => "rect",
        R::Trapezoid { .. } => "trapezoid",
        R::StraightHall { .. } => "hall",
        R::ArcHall { .. } => "archall",
        R::HexCell { .. } => "hexcell",
    }
}

/// How many times `other.contains` flips along `sh`'s whole border. Two means the borders
/// overlap in one span — the well-defined clip. Zero means they never meet (or one is wholly
/// inside the other); more than two means several spans.
fn border_flips(sh: &R, other: &R) -> usize {
    let Some(per) = sh.perimeter() else {
        return usize::MAX;
    };
    let n = 720;
    let inside: Vec<bool> = (0..n)
        .map(|i| other.contains(sh.wall_point(per * i as f64 / n as f64)))
        .collect();
    (0..n).filter(|&i| inside[i] != inside[(i + 1) % n]).count()
}

/// How many times a polyline crosses `sh`'s border, by sign flips of `contains` along it.
/// Open polyline, so ends are not joined.
fn poly_flips(poly: &[P], sh: &R) -> usize {
    if poly.len() < 2 {
        return 0;
    }
    // Sample each segment, so a segment that dips through the border is not missed.
    let mut inside = Vec::new();
    for w in poly.windows(2) {
        let (a, b) = (w[0], w[1]);
        let steps = ((a.0 - b.0).hypot(a.1 - b.1) / 1.0).ceil().max(1.0) as usize;
        for t in 0..steps {
            let f = t as f64 / steps as f64;
            inside.push(sh.contains((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)));
        }
    }
    inside.push(sh.contains(*poly.last().unwrap()));
    (0..inside.len() - 1)
        .filter(|&i| inside[i] != inside[i + 1])
        .count()
}

/// Unordered pairs of shaped areas whose ROOM cells touch. Growth keeps a one-cell rock gap
/// between areas unless they fused, so adjacency at distance 1 is the fusion test.
fn fused_pairs(m: &CaveMap) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let cells: Vec<Vec<maps_core::grid::Hex>> = (0..m.areas.count())
        .map(|i| m.areas.room_cells(i).collect())
        .collect();
    for i in 0..m.areas.count() {
        for j in (i + 1)..m.areas.count() {
            let touch = cells[i]
                .iter()
                .any(|a| cells[j].iter().any(|b| a.distance(*b) == 1));
            if touch {
                out.push((i, j));
            }
        }
    }
    out
}

/// Every connector stretch in a wall run, as (polyline, the room shapes flanking it).
fn connector_stretches(m: &CaveMap) -> Vec<(Vec<P>, Vec<R>)> {
    let mut out = Vec::new();
    for run in &m.dungeon_walls {
        let mut i = 0;
        while i < run.len() {
            if is_room(&run[i].1) {
                i += 1;
                continue;
            }
            let sh = run[i].1;
            let mut j = i;
            while j + 1 < run.len() && run[j + 1].1 == sh {
                j += 1;
            }
            let mut flank = Vec::new();
            if i > 0 && is_room(&run[i - 1].1) {
                flank.push(run[i - 1].1);
            }
            if j + 1 < run.len() && is_room(&run[j + 1].1) {
                flank.push(run[j + 1].1);
            }
            if !flank.is_empty() {
                out.push((run[i..=j].iter().map(|v| v.0).collect(), flank));
            }
            i = j + 1;
        }
    }
    out
}

#[test]
fn openings() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    for tagstr in [
        "large,ruins,dungeon,fused",
        "large,chamber,connected,ruins,dungeon,truchet",
    ] {
        for (lvl, lv) in [("defaults", None), ("dense", Some(1.0))] {
            // pair-class -> (one span, never meet, several spans)
            let mut pairs: std::collections::BTreeMap<String, [usize; 3]> = Default::default();
            // connector flank crossings -> count
            let mut conn: std::collections::BTreeMap<usize, usize> = Default::default();
            let mut conn_kind: std::collections::BTreeMap<(String, usize), usize> =
                Default::default();
            for seed in 1..=seeds {
                let m = generate_with(
                    seed,
                    &GenOptions {
                        tags: Some(Tags::parse(tagstr).unwrap()),
                        ruins_level: lv,
                        dungeon_level: lv,
                        fuse_level: lv,
                        ..GenOptions::default()
                    },
                );
                for (i, j) in fused_pairs(&m) {
                    let (Some(a), Some(b)) = (
                        m.ruins.get(i).copied().flatten(),
                        m.ruins.get(j).copied().flatten(),
                    ) else {
                        continue;
                    };
                    if !is_room(&a) || !is_room(&b) {
                        continue;
                    }
                    let mut k = [kind(&a), kind(&b)];
                    k.sort();
                    let f = border_flips(&a, &b);
                    let slot = pairs.entry(format!("{}-{}", k[0], k[1])).or_default();
                    match f {
                        2 => slot[0] += 1,
                        0 => slot[1] += 1,
                        _ => slot[2] += 1,
                    }
                }
                for (poly, flank) in connector_stretches(&m) {
                    for room in flank {
                        let f = poly_flips(&poly, &room);
                        *conn.entry(f).or_default() += 1;
                        // For 0 flips, WHICH side: wholly outside the room (the wall never
                        // reaches its border) or wholly inside it?
                        let tag = if f == 0 {
                            let ends_in =
                                room.contains(poly[0]) || room.contains(*poly.last().unwrap());
                            if ends_in { "0-inside" } else { "0-outside" }
                        } else if f == 1 {
                            "1-crosses"
                        } else {
                            "many"
                        };
                        *conn_kind
                            .entry((format!("{}/{tag}", kind(&room)), f))
                            .or_default() += 1;
                    }
                }
            }
            assert_eq!(
                pairs.values().map(|v| v[2]).sum::<usize>(),
                0,
                "[{tagstr} {lvl}] a fused pair's borders overlap in several spans, so the clip \
                 has no single answer: {pairs:?}"
            );
            let many: usize = conn.iter().filter(|&(&f, _)| f > 1).map(|(_, n)| n).sum();
            assert_eq!(
                many, 0,
                "[{tagstr} {lvl}] {many} connector wall stretch(es) cross a room's border more \
                 than once, so one throat would need several disjoint cuts: {conn:?}"
            );
            println!("=== {tagstr} {lvl} ({seeds} seeds)");
            for (k, v) in &pairs {
                println!(
                    "    fused pair {k:15} one-span={:4} never-meet={:4} several={:3}",
                    v[0], v[1], v[2]
                );
            }
            let tot: usize = conn.values().sum();
            println!("    connector x room ({tot} flanks): {conn:?}");
            println!("      by room kind: {conn_kind:?}");
        }
    }
}
