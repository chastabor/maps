//! Side-by-side preview of the two ring-gap policies phase 3 has to choose between
//! (`plans/tile-corridor-render.md`, open question 2).
//!
//! Both panels cut the same corridor attachments out of the same room borders — the only
//! difference is how much of each attachment becomes opening:
//!
//! * **full contact** — the gap is `corridor::Gap::span` entire, so a wide passage gets a wide
//!   opening. This is what the render cuts TODAY: `doorway::jambs` takes the porch chord's
//!   projected arc, which is the contact.
//! * **lattice door** — `corridor::Gap::opening`: one apothem per PAIR of touched edges, centred on
//!   the contact, so wall survives either side of a wide contact. **The user's choice**, and a
//!   narrowing relative to today.
//!
//! Drawn through `clip::wall_spans`, so what you see is the wall the render would actually draw:
//! thick dark stroke is surviving wall, the dotted line is where the border runs underneath, and
//! the gap is the absence between them. Green is the leaf — `Gap::leaf`, the straight chord of the
//! opening, angled across a rect corner and flat across a circle's arc.
//!
//! ```text
//! SEED=22 TAGS=large,ruins,dungeon,fused OUT=policy.svg \
//!     cargo run -p maps-core --release --example gap_policy
//! ```

mod common;

use maps_core::clip::{Span, wall_spans};
use maps_core::corridor::{Gap, corridors, door_leaf};
use maps_core::grid::HEX_SIZE as S;
use maps_core::outline::wall_walk;
use maps_core::render::hex_points;
use maps_core::ruins::RuinShape;
use maps_core::{AreaKind, CaveMap};
use std::fmt::Write as _;

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    FullContact,
    LatticeDoor,
}

/// One attachment: the room it lands on and the gap it covers.
struct Attach {
    room: usize,
    gap: Gap,
}

fn attachments(m: &CaveMap) -> Vec<Attach> {
    let mut out = Vec::new();
    for cor in corridors(&m.areas, &m.topology.connections, S) {
        for (land, room) in cor.attach.iter().zip([cor.a, cor.b]) {
            for g in &land.gaps {
                out.push(Attach { room, gap: *g });
            }
        }
    }
    out
}

/// The opening this policy cuts for one attachment. The lattice-door case is
/// [`Gap::opening`] — the policy has one home in the crate and this only chooses between it and
/// opening the contact whole.
fn opening(a: &Attach, per: f64, policy: Policy) -> Span {
    match policy {
        Policy::FullContact => a.gap.span,
        Policy::LatticeDoor => a.gap.opening(per, S),
    }
}

/// A span as an SVG path, walked corner-exactly (`outline::wall_walk` emits every wall corner
/// inside the span, where uniform sampling can step over one).
fn arc(sh: &RuinShape, span: Span, per: f64) -> String {
    let closed = span.len >= per - 1e-9;
    let mut d = String::new();
    for (i, p) in wall_walk(sh, span.from, 1.0, span.len, S, closed)
        .into_iter()
        .enumerate()
    {
        let _ = write!(d, "{}{:.2} {:.2}", if i == 0 { "M" } else { "L" }, p.0, p.1);
    }
    if closed {
        d.push('Z');
    }
    d
}

/// The per-room scaffolding `panel` and `survey` share: the border, its perimeter, this room's
/// attachments and the openings the policy cuts for them.
fn room_openings<'a>(
    m: &CaveMap,
    atts: &'a [Attach],
    i: usize,
    policy: Policy,
) -> Option<(RuinShape, f64, Vec<&'a Attach>, Vec<Span>)> {
    if m.areas.kind(i) != AreaKind::Dungeon {
        return None;
    }
    let (sh, per) = m.areas.room_border_per(i)?;
    let mine: Vec<&Attach> = atts.iter().filter(|a| a.room == i).collect();
    let openings: Vec<Span> = mine.iter().map(|a| opening(a, per, policy)).collect();
    Some((sh, per, mine, openings))
}

fn panel(m: &CaveMap, policy: Policy) -> String {
    let mut s = String::new();
    let atts = attachments(m);
    // Corridor floor, for context: the tiles the attachments come from.
    s.push_str(r##"<g stroke="none" fill="#e8e4d8">"##);
    for c in &m.topology.connections {
        for &h in &c.along {
            let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
        }
    }
    s.push_str("</g>");
    for i in 0..m.areas.count() {
        let Some((sh, per, _, openings)) = room_openings(m, &atts, i, policy) else {
            continue;
        };
        // Where the border runs, dotted — so the gap reads as absence of wall, not as nothing.
        let _ = write!(
            s,
            r##"<path d="{}" fill="none" stroke="#b9b2a0" stroke-width="0.7" stroke-dasharray="2 2"/>"##,
            arc(
                &sh,
                Span {
                    from: 0.0,
                    len: per
                },
                per
            )
        );
        // The wall the render would draw: border minus the openings.
        for w in wall_spans(&sh, &openings, 0.4) {
            let _ = write!(
                s,
                r##"<path d="{}" fill="none" stroke="#2f2a24" stroke-width="3.2" stroke-linecap="butt"/>"##,
                arc(&sh, w, per)
            );
        }
        // The leaf: the STRAIGHT CHORD of the gap — a segment joining the two points where the
        // wall stops. Not the arc (a circle's door is a flat leaf across its opening, not a curved
        // one) and not clamped to one edge (a gap that turns a rect corner gets one angled door
        // across the corner, never an L). One rule, both shapes, no special case.
        for span in &openings {
            let (p0, p1) = door_leaf(&sh, *span, per);
            let _ = write!(
                s,
                r##"<path d="M{:.2} {:.2}L{:.2} {:.2}" fill="none" stroke="#2fa84f" stroke-width="2.8" stroke-linecap="round"/>"##,
                p0.0, p0.1, p1.0, p1.1
            );
        }
    }
    s
}

/// The trade-off the two panels show, as numbers over a seed range.
///
/// The two costs pull opposite ways, which is the whole reason this is a policy question:
/// * `wall_over_floor` — surviving wall standing on corridor floor. This is the plan's own
///   "passage wider than its wall gaps" defect (the `widen_mouths` row of the why-replace table):
///   a wall drawn across ground the passage occupies. Full contact makes it zero by construction.
/// * `wall_removed` and `corners_lost` — how much of each room's silhouette the openings eat. A
///   rect corner strictly inside an opening means the room loses its corner, and with it the
///   reading of where the room ends.
fn survey(seeds: u64, tags: &str) {
    for (policy, name) in [
        (Policy::FullContact, "full contact"),
        (Policy::LatticeDoor, "lattice door"),
    ] {
        let (mut over, mut removed, mut corners, mut n) = (0.0f64, 0.0f64, 0usize, 0usize);
        for seed in 1..=seeds {
            let m = common::generate(seed, tags);
            let floor: std::collections::HashSet<maps_core::grid::Hex> = m
                .topology
                .connections
                .iter()
                .flat_map(|c| c.along.iter().copied())
                .collect();
            let atts = attachments(&m);
            for i in 0..m.areas.count() {
                let Some(sh) = m.areas.room_border(i) else {
                    continue;
                };
                let Some(per) = sh.perimeter() else { continue };
                let mine: Vec<&Attach> = atts.iter().filter(|a| a.room == i).collect();
                if mine.is_empty() {
                    continue;
                }
                n += mine.len();
                let openings: Vec<Span> = mine.iter().map(|a| opening(a, per, policy)).collect();
                for o in &openings {
                    removed += o.len;
                    // A rect corner strictly inside this opening.
                    if let RuinShape::Rect { hw, hh, .. } = sh {
                        let (w, hgt) = (2.0 * hw, 2.0 * hh);
                        for t in [0.0, w, w + hgt, 2.0 * w + hgt] {
                            if o.contains(t, per)
                                && (t - o.from).rem_euclid(per) > 1e-6
                                && (t - o.end(per)).rem_euclid(per) > 1e-6
                            {
                                corners += 1;
                            }
                        }
                    }
                }
                // Surviving wall standing over corridor floor, sampled every ~1px.
                for wsp in wall_spans(&sh, &openings, 0.4) {
                    let steps = wsp.len.ceil().max(1.0) as usize;
                    for k in 0..steps {
                        let t =
                            (wsp.from + wsp.len * (k as f64 + 0.5) / steps as f64).rem_euclid(per);
                        let p = sh.wall_point(t);
                        if floor.contains(&maps_core::grid::Hex::at(p, S)) {
                            over += wsp.len / steps as f64;
                        }
                    }
                }
            }
        }
        println!(
            "  {name:<13} attachments {n:5}  wall over floor {over:8.0}px               wall removed {removed:8.0}px  rect corners lost {corners:4}"
        );
    }
}

fn main() {
    if let Ok(v) = std::env::var("SEEDS") {
        let seeds: u64 = v.parse().unwrap_or(60);
        for tags in common::CONFIGS {
            println!("=== {tags} ({seeds} seeds)");
            survey(seeds, tags);
        }
        return;
    }
    let seed: u64 = common::env("SEED", 22);
    let tag_str = common::tags_env();
    let out = std::env::var("OUT").unwrap_or_else(|_| "gap_policy.svg".to_string());
    let m = common::generate(seed, &tag_str);
    // Bounds over everything drawn: dungeon borders and corridor tiles.
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for c in &m.topology.connections {
        for &h in &c.along {
            pts.extend(h.corners(S));
        }
    }
    for i in 0..m.areas.count() {
        if m.areas.kind(i) != AreaKind::Dungeon {
            continue;
        }
        if let Some(sh) = m.areas.room_border(i) {
            let per = sh.perimeter().unwrap_or(0.0);
            for k in 0..64 {
                pts.push(sh.wall_point(per * k as f64 / 64.0));
            }
        }
    }
    if pts.is_empty() {
        eprintln!("seed {seed} [{tag_str}] has no dungeon rooms to draw");
        return;
    }
    let pad = 2.0 * S;
    let ((lo_x, lo_y), (hi_x, hi_y)) = common::bounds(&pts);
    let (w, h) = (hi_x - lo_x + 2.0 * pad, hi_y - lo_y + 2.0 * pad);
    let gutter = 1.5 * S;
    let total = 2.0 * w + gutter;
    let label_h = 3.0 * S;
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total:.1} {:.1}" width="{:.0}" height="{:.0}"><rect width="100%" height="100%" fill="#f4efe2"/>"##,
        h + label_h,
        total * 2.2,
        (h + label_h) * 2.2,
    );
    for (k, (policy, title)) in [
        (Policy::FullContact, "gap = FULL CONTACT"),
        (Policy::LatticeDoor, "gap = LATTICE DOOR centred on contact"),
    ]
    .into_iter()
    .enumerate()
    {
        let dx = k as f64 * (w + gutter);
        let _ = write!(
            svg,
            r##"<text x="{:.1}" y="{:.1}" font-family="sans-serif" font-size="{:.1}" font-weight="bold" fill="#2f2a24">{title}</text>"##,
            dx + pad,
            label_h * 0.7,
            S * 1.3
        );
        let _ = write!(
            svg,
            r##"<g transform="translate({:.2} {:.2})">{}</g>"##,
            dx + pad - lo_x,
            label_h + pad - lo_y,
            panel(&m, policy)
        );
    }
    svg.push_str("</svg>");
    std::fs::write(&out, svg).expect("write svg");
    println!("seed {seed}  tags {tag_str} -> {out}");
}
