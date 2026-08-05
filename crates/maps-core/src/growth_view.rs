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

    // Tiles their own area's shape fails to bound — the defect this view exists to expose.
    //
    // The test is per shape kind, because the two are supposed to sit differently relative to
    // their tiles (see `plans/tile-first-render.md`):
    //
    // - a **circle** contains its tiles, so a tile with any vertex outside is wrong;
    // - a **rect**'s border belongs *inside* its tiles — sides down the outer column, top and
    //   bottom joining the shoulder vertices — so an overhang is by design, and only a tile
    //   lying WHOLLY outside the rect is wrong.
    //
    // Flagging a rect's overhang would make this count impossible to drive to zero, which is
    // the whole point of drawing it.
    s.push_str(r##"<g fill="none" stroke="#ff2d2d" stroke-width="1.6">"##);
    for i in 0..map.areas.count() {
        let Some(sh) = map.areas.shape(i) else {
            continue;
        };
        // ROOM cells, which is what the shape was derived from. Corridor floor sits outside
        // the room's own border by design — the connector walls it — and is already marked
        // white above, so flagging it here would only ever be noise.
        for h in map.areas.room_cells(i) {
            let c = h.center(S);
            let mut vs = (0..6).map(|k| crate::grid::hex_corner(c, k, S));
            let unbounded = match sh {
                RuinShape::Rect { .. } => !vs.any(|v| sh.contains(v)) && !sh.contains(c),
                _ => vs.any(|v| !sh.contains(v)),
            };
            if unbounded {
                let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
            }
        }
    }
    s.push_str("</g>");

    // Door cells and exit stubs: floor that belongs to the topology rather than to an area,
    // and the tiles a passage attaches through.
    s.push_str(r##"<g stroke="none">"##);
    for d in &map.topology.connections {
        // The whole run — a connection is CONNECTION_WIDTH wide plus its apron, and the
        // corridor overlay below draws its arrows over exactly these tiles.
        for &h in &d.along {
            let _ = write!(
                s,
                r##"<polygon points="{}" fill="#f2f2ea" fill-opacity="0.9"/>"##,
                hex_points(h)
            );
        }
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

    // Each fused pair's COMPOUND wall: the arc of one border outside the other, then the
    // reverse — what phase 2a will draw in place of a chord between two projected run ends.
    // Drawn over both rooms' own outlines so the difference is visible: where this leaves the
    // cyan/amber circles, those stretches are the interior barrier the wall should not include,
    // and the seam corner here is the one the finished render currently chamfers.
    //
    // Fusion is read geometrically — growth keeps a one-cell rock gap between every pair it did
    // not fuse, so room tiles at distance 1 mean the gap was closed. `compound_wall` declines
    // any pair it does not answer for (no crossings, nested, a hall), and those simply draw
    // nothing rather than a guess.
    s.push_str(r##"<g fill="none" stroke="#ff53d0" stroke-width="2.0" stroke-opacity="0.95">"##);
    for a in 0..map.areas.count() {
        let Some(sa) = map.areas.shape(a) else {
            continue;
        };
        for b in (a + 1)..map.areas.count() {
            let Some(sb) = map.areas.shape(b) else {
                continue;
            };
            let bs: Vec<Hex> = map.areas.room_cells(b).collect();
            let fused = map
                .areas
                .room_cells(a)
                .any(|x| bs.iter().any(|y| x.distance(*y) <= 1));
            if !fused {
                continue;
            }
            if let Some(run) = crate::outline::compound_wall(sa, sb, S) {
                let pts: Vec<String> = run
                    .iter()
                    .map(|&(p, _)| format!("{:.1},{:.1}", p.0, p.1))
                    .collect();
                let _ = write!(s, r##"<polyline points="{}"/>"##, pts.join(" "));
            }
        }
    }
    s.push_str("</g>");

    corridor_overlay(map, &mut s);

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

/// A head at `to`, aimed along `from -> to`.
fn arrow_head(s: &mut String, from: (f64, f64), to: (f64, f64)) {
    let d = (to.0 - from.0, to.1 - from.1);
    let len = d.0.hypot(d.1).max(1e-6);
    let (u, n) = ((d.0 / len, d.1 / len), (-d.1 / len, d.0 / len));
    for sgn in [1.0, -1.0] {
        let _ = write!(
            s,
            r##"<path d="M{} {}L{} {}"/>"##,
            d1(to.0 - u.0 * 3.0 + n.0 * 3.0 * sgn),
            d1(to.1 - u.1 * 3.0 + n.1 * 3.0 * sgn),
            d1(to.0),
            d1(to.1)
        );
    }
}

/// A double-headed straight arrow.
fn arrow(s: &mut String, from: (f64, f64), to: (f64, f64)) {
    let _ = write!(
        s,
        r##"<path d="M{} {}L{} {}"/>"##,
        d1(from.0),
        d1(from.1),
        d1(to.0),
        d1(to.1)
    );
    arrow_head(s, from, to);
    arrow_head(s, to, from);
}

/// Corridor overlay — `plans/tile-corridor-render.md` phases 0-1.
///
/// The acceptance surface for `corridor::corridors`, compared arrow-for-arrow against the
/// hand-annotated reference (`samples/grow-tile-render.png`). Everything here is READ from
/// the model — the overlay validates the derivation, so it must not re-derive any fact
/// itself. Yellow double arrows are per-tile axes (straight lanes when an opposite pairing
/// exists, one bent arrow otherwise, bending into a tile's collapse corner per R3); red
/// marks are the landings; green is the phase-1 spine.
fn corridor_overlay(map: &CaveMap, s: &mut String) {
    use crate::corridor::{Mark, TileAxis, corridors};
    let cors = corridors(&map.areas, &map.topology.connections, S);
    s.push_str(r##"<g stroke="#ffd23f" stroke-width="1.4" fill="none" stroke-linecap="round">"##);
    let mut marks = String::new();
    for cor in &cors {
        for (tile, ax) in cor.tiles.iter().zip(&cor.axes) {
            let c = tile.center(S);
            let corners = tile.corners(S);
            // The midpoint of the edge facing neighbour `k` — via `edge_corners`, since edge
            // `k` faces neighbour `(6-k)%6`, not `k`.
            let mid = |k: usize| {
                let (e0, e1) = Hex::edge_corners(k);
                (
                    (corners[e0].0 + corners[e1].0) / 2.0,
                    (corners[e0].1 + corners[e1].1) / 2.0,
                )
            };
            let (col_a, col_b) = (
                TileAxis::collapse(&ax.touch_a).map(|c| corners[c]),
                TileAxis::collapse(&ax.touch_b).map(|c| corners[c]),
            );
            // A bent arrow's endpoint: the tile's collapse corner when this side touches
            // the room and the contact collapsed, else the edge midpoint.
            let end = |k: usize, touch: &[bool; 6], col: Option<(f64, f64)>| {
                if touch[k] {
                    col.unwrap_or(mid(k))
                } else {
                    mid(k)
                }
            };
            // R1: every opposite pairing is a straight LANE at the edge midpoints, even
            // when room contact is multi-side. R2: otherwise ONE bent arrow through the
            // centre, widest span winning, bending into collapse corners (R3).
            let straights: Vec<(usize, usize)> = (0..6)
                .filter(|&k| ax.toward_a[k] && ax.toward_b[(k + 3) % 6])
                .map(|k| (k, (k + 3) % 6))
                .collect();
            if straights.is_empty() {
                type Bend = ((f64, f64), (f64, f64), f64);
                let mut best: Option<Bend> = None;
                for k in (0..6).filter(|&k| ax.toward_a[k]) {
                    for m in (0..6).filter(|&m| ax.toward_b[m]) {
                        let pa = end(k, &ax.touch_a, col_a);
                        let pb = end(m, &ax.touch_b, col_b);
                        let d = (pa.0 - pb.0).hypot(pa.1 - pb.1);
                        if best.is_none_or(|(_, _, bd)| d > bd) {
                            best = Some((pa, pb, d));
                        }
                    }
                }
                if let Some((pa, pb, _)) = best {
                    let _ = write!(
                        s,
                        r##"<path d="M{} {}L{} {}L{} {}"/>"##,
                        d1(pa.0),
                        d1(pa.1),
                        d1(c.0),
                        d1(c.1),
                        d1(pb.0),
                        d1(pb.1)
                    );
                    arrow_head(s, c, pa);
                    arrow_head(s, c, pb);
                }
            } else {
                for (k, m) in straights {
                    arrow(s, mid(k), mid(m));
                }
            }
        }
        for att in &cor.attach {
            for (_, m) in att {
                match m {
                    Mark::Point(p) => {
                        let _ = write!(
                            marks,
                            r##"<circle cx="{}" cy="{}" r="2.4" fill="#ff2d2d" stroke="none"/>"##,
                            d1(p.0),
                            d1(p.1)
                        );
                    }
                    Mark::Bar(p, q) => {
                        let _ = write!(
                            marks,
                            r##"<path d="M{} {}L{} {}" stroke="#ff2d2d" stroke-width="2.2" fill="none"/>"##,
                            d1(p.0),
                            d1(p.1),
                            d1(q.0),
                            d1(q.1)
                        );
                    }
                }
            }
        }
    }
    s.push_str("</g>");
    s.push_str(&marks);
    // Phase 1: the corridor spine. Green so it reads over the yellow arrows; dots at the
    // waypoints. Must be continuous, inside the corridor's tiles, and end on both fitted
    // borders — judged by eye against the same reference.
    s.push_str(r##"<g stroke="#37e6a0" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-opacity="0.95">"##);
    for cor in &cors {
        if cor.centerline.len() < 2 {
            continue;
        }
        let mut d = String::new();
        for (k, p) in cor.centerline.iter().enumerate() {
            let _ = write!(
                d,
                "{}{} {}",
                if k == 0 { "M" } else { "L" },
                d1(p.0),
                d1(p.1)
            );
        }
        let _ = write!(s, r##"<path d="{d}"/>"##);
    }
    s.push_str("</g>");
    // Phase 2: the walls offset from each spine, straightened onto apothem lines and capped
    // on the room borders. White (the compound-wall layer above already owns magenta), so a
    // wall crossing a tile it should have cleared, or a cap missing its border, is obvious
    // against the tiles underneath.
    s.push_str(r##"<g stroke="#ffffff" stroke-width="2.2" fill="none" stroke-linejoin="round">"##);
    for cor in &cors {
        for side in cor.walls(&map.areas, S) {
            if side.len() < 2 {
                continue;
            }
            let mut d = String::new();
            for (k, p) in side.iter().enumerate() {
                let _ = write!(
                    d,
                    "{}{} {}",
                    if k == 0 { "M" } else { "L" },
                    d1(p.0),
                    d1(p.1)
                );
            }
            let _ = write!(s, r##"<path d="{d}"/>"##);
        }
    }
    s.push_str("</g>");
    for cor in &cors {
        for p in &cor.centerline {
            let _ = write!(
                s,
                r##"<circle cx="{}" cy="{}" r="1.6" fill="#37e6a0" stroke="none"/>"##,
                d1(p.0),
                d1(p.1)
            );
        }
    }
}
