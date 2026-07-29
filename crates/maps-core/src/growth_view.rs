//! Growth-only view: the tiles, which area owns each, and the shape derived from them.
//!
//! Deliberately **not** the finished map. Nothing here traces a boundary, smooths, projects a
//! vertex onto a shape, or draws a wall band — the whole reason this view exists is to see what
//! `growth` handed forward, before any of that. When the drawn map and this view disagree, the
//! disagreement is in the render pipeline, not in growth.
//!
//! Separate from [`render::debug_svg`](crate::render::debug_svg) on purpose: that one is folded
//! into the sweep's content digests, so changing it would forfeit the byte-identity guard the
//! plans rely on.
//!
//! What it shows, and why each is worth seeing:
//!
//! - **Tiles by area**, filled from the same palette and labelled with the same `5D/v4hf` tags
//!   the finished render uses, so a room can be cross-referenced between the two views.
//! - **The derived shape** stroked over its own tiles. A shape is supposed to describe the tiles
//!   it came from; drawing both together is the only way to see whether it does.
//! - **Tiles their own shape does not contain**, outlined in red. This is the defect class behind
//!   most of `plans/fuse-case-taxonomy.md`: a fitted circle stops at
//!   `max_cell_centre_distance + 0.4·s` and a rect's top and bottom sit at `±s/2` while a
//!   pointy-top tile reaches `±s`, so floor routinely lies outside its own room's wall.
//! - **Tile roles** — corridor floor (`join`), ground given back (`eroded`), door cells and exit
//!   stubs — because growth's output is more than "which area owns what".

use crate::grid::Hex;
use crate::ruins::RuinShape;
use crate::{AreaKind, CaveMap};
use std::fmt::Write;

/// Hex size the view draws at. Matches `render`'s so coordinates line up between views.
const S: f64 = 12.0;
const MARGIN: f64 = 20.0;

/// Distinct fills per area, reused from the debug palette's spirit: enough hues that
/// neighbours rarely collide, and stable under the area index.
const PALETTE: [&str; 12] = [
    "#e6194b", "#3cb44b", "#ffe119", "#4363d8", "#f58231", "#911eb4", "#42d4f4", "#f032e6",
    "#bfef45", "#fabed4", "#469990", "#dcbeff",
];

fn d1(v: f64) -> String {
    format!("{:.1}", v)
}

fn hex_points(h: Hex) -> String {
    h.corners(S)
        .iter()
        .map(|(x, y)| format!("{:.2},{:.2}", x, y))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A stable 4-char code for an area, hashed from its sorted cells — the same scheme the
/// finished render labels with, so `5D/v4hf` means the same room in both views.
fn area_hash(cells: &[Hex]) -> String {
    let mut v: Vec<(i32, i32)> = cells.iter().map(|c| (c.q, c.r)).collect();
    v.sort_unstable();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (q, r) in v {
        for b in q.to_le_bytes().iter().chain(r.to_le_bytes().iter()) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = [0u8; 4];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(h % 36) as usize];
        h /= 36;
    }
    String::from_utf8(out.to_vec()).unwrap()
}

/// The shape's outline as an SVG element, in its own coordinates — no offsetting, no band.
fn shape_path(sh: RuinShape, stroke: &str, width: f64) -> String {
    let common = format!(r##"fill="none" stroke="{stroke}" stroke-width="{width}""##);
    match sh {
        RuinShape::Circle { cx, cy, r } => format!(
            r##"<circle cx="{}" cy="{}" r="{}" {common}/>"##,
            d1(cx),
            d1(cy),
            d1(r)
        ),
        RuinShape::Rect { cx, cy, hw, hh } => format!(
            r##"<rect x="{}" y="{}" width="{}" height="{}" {common}/>"##,
            d1(cx - hw),
            d1(cy - hh),
            d1(2.0 * hw),
            d1(2.0 * hh)
        ),
        RuinShape::StraightHall { ax, ay, bx, by, hw } => format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="{}" stroke-opacity="0.35" fill="none"/>"##,
            d1(ax),
            d1(ay),
            d1(bx),
            d1(by),
            d1(2.0 * hw)
        ),
        RuinShape::Trapezoid { wall0, wall1 } => format!(
            r##"<polygon points="{},{} {},{} {},{} {},{}" {common}/>"##,
            d1(wall0.0.0),
            d1(wall0.0.1),
            d1(wall0.1.0),
            d1(wall0.1.1),
            d1(wall1.1.0),
            d1(wall1.1.1),
            d1(wall1.0.0),
            d1(wall1.0.1)
        ),
        RuinShape::ArcHall { cx, cy, r, hw } => format!(
            r##"<circle cx="{}" cy="{}" r="{}" stroke="{stroke}" stroke-width="{}" stroke-opacity="0.35" fill="none"/>"##,
            d1(cx),
            d1(cy),
            d1(r),
            d1(2.0 * hw)
        ),
        RuinShape::HexCell { cx, cy, s } => format!(
            r##"<circle cx="{}" cy="{}" r="{}" {common}/>"##,
            d1(cx),
            d1(cy),
            d1(s)
        ),
    }
}

/// Render growth's output: tiles, their owning area, and the shape derived from them.
///
/// See the module docs for what each layer means. `labels` adds the per-area `5D/v4hf` tag.
pub fn growth_svg(map: &CaveMap, labels: bool) -> String {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &h in map.grid.cells() {
        for (x, y) in h.corners(S) {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let (vx, vy) = (min_x - MARGIN, min_y - MARGIN);
    let (vw, vh) = (max_x - min_x + 2.0 * MARGIN, max_y - min_y + 2.0 * MARGIN);
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{:.0}" height="{:.0}">"##,
        d1(vx),
        d1(vy),
        d1(vw),
        d1(vh),
        vw,
        vh
    );
    let _ = write!(
        s,
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#14141c"/>"##,
        d1(vx),
        d1(vy),
        d1(vw),
        d1(vh)
    );

    // The bare lattice, so empty rock still reads as tiles.
    s.push_str(r##"<g stroke="#282833" stroke-width="0.5" fill="none">"##);
    for &h in map.grid.cells() {
        let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
    }
    s.push_str("</g>");

    // Owned tiles, per area. `cells` is the footprint, so eroded ground appears too — drawn
    // faint, since it is a record of where the area was rather than floor it still holds.
    for i in 0..map.areas.count() {
        let color = PALETTE[i % PALETTE.len()];
        for &h in &map.areas.cells[i] {
            let (op, stroke) = if map.areas.is_eroded(h) {
                (0.18, "none")
            } else if map.areas.join().contains(&h) {
                (0.75, "#ffffff")
            } else {
                (0.62, "none")
            };
            let _ = write!(
                s,
                r##"<polygon points="{}" fill="{color}" fill-opacity="{op}" stroke="{stroke}" stroke-width="0.7"/>"##,
                hex_points(h)
            );
        }
    }

    // Tiles their OWN shape fails to contain — the defect this view exists to expose.
    s.push_str(r##"<g fill="none" stroke="#ff2d2d" stroke-width="1.6">"##);
    for i in 0..map.areas.count() {
        let Some(sh) = map.areas.shape(i) else {
            continue;
        };
        for h in map.areas.floor_cells(i) {
            let c = h.center(S);
            let escapes = (0..6)
                .map(|k| crate::grid::hex_corner(c, k, S))
                .any(|v| !sh.contains(v));
            if escapes {
                let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
            }
        }
    }
    s.push_str("</g>");

    // Door cells and exit stubs: floor that belongs to the topology rather than to an area,
    // and the tiles a passage attaches through.
    s.push_str(r##"<g stroke="none">"##);
    for d in &map.topology.doors {
        let _ = write!(
            s,
            r##"<polygon points="{}" fill="#f2f2ea" fill-opacity="0.9"/>"##,
            hex_points(d.cell)
        );
    }
    for e in &map.topology.exits {
        for &h in &e.stub {
            let _ = write!(
                s,
                r##"<polygon points="{}" fill="#ff8c42" fill-opacity="0.85"/>"##,
                hex_points(h)
            );
        }
    }
    s.push_str("</g>");

    // Each area's derived shape, over its own tiles. Cyan for a dungeon, amber for a ruin —
    // the two kinds that carry geometry.
    for i in 0..map.areas.count() {
        let Some(sh) = map.areas.shape(i) else {
            continue;
        };
        let stroke = match map.areas.kind(i) {
            AreaKind::Dungeon => "#31d2f2",
            _ => "#ffb000",
        };
        s.push_str(&shape_path(sh, stroke, 1.3));
    }

    if labels {
        let _ = write!(
            s,
            r##"<g font-family="Menlo, Consolas, monospace" text-anchor="middle" paint-order="stroke" stroke="#14141c" stroke-width="3" stroke-linejoin="round">"##
        );
        for i in 0..map.areas.count() {
            let cells: Vec<Hex> = map.areas.floor_cells(i).collect();
            if cells.is_empty() {
                continue;
            }
            let (mut cx, mut cy) = (0.0, 0.0);
            for &h in &cells {
                let p = h.center(S);
                cx += p.0;
                cy += p.1;
            }
            let n = cells.len() as f64;
            let (cx, cy) = (cx / n, cy / n);
            let kind = match map.areas.kind(i) {
                AreaKind::Organic => 'O',
                AreaKind::Ruin => 'R',
                AreaKind::Dungeon => 'D',
            };
            let _ = write!(
                s,
                r##"<text x="{}" y="{}" font-size="14" font-weight="bold" fill="#ffffff">{}{kind}</text>"##,
                d1(cx),
                d1(cy),
                i + 1
            );
            let _ = write!(
                s,
                r##"<text x="{}" y="{}" font-size="10" fill="#c9c9d4">{}</text>"##,
                d1(cx),
                d1(cy + 12.0),
                area_hash(&cells)
            );
        }
        s.push_str("</g>");
    }

    s.push_str("</svg>");
    s
}
