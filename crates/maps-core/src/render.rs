//! SVG renderers: `svg` draws the finished map from the smoothed outline;
//! `debug_svg` shows the raw hex cells, one colour per area.

use crate::grid::Hex;
use crate::outline::Point;
use crate::{AreaKind, CaveMap, DoorStyle, GridStyle, Mode};
use std::collections::HashSet;
use std::fmt;
use std::fmt::Write;

/// Colour scheme, chosen per map mode.
struct Style {
    /// The deepest layer of all — outside every tree band. Forest keeps it
    /// darkest; cave sets all depth colours to the same beige family, which
    /// is why no depth shading shows there.
    bg: &'static str,
    floor: &'static str,
    line: &'static str,
    /// Outline colour for tree canopies.
    tree_line: &'static str,
    stone: &'static str,
    /// Canopy shades by depth band: nearest the clearing first (lightest),
    /// receding toward the background colour.
    tree_shades: [&'static str; 3],
    /// Soft dark band hugging the outside of the wall (cave mode's nearest,
    /// darkest layer — the forest sets it to its background).
    shadow: &'static str,
    /// Ink colour for the wall hatch fans.
    hatch: &'static str,
    /// Fill for ruin masonry tiles (forest mode).
    tile: &'static str,
    /// Ruin floor mosaic shades, subtle variations on the floor colour.
    mosaic: [&'static str; crate::decor::MOSAIC_SHADES],
    /// Stroke for line-based ruin floor patterns (truchet/islamic).
    pattern_line: &'static str,
    title: &'static str,
    water: &'static str,
    deep: &'static str,
    mud: &'static str,
}

const CAVE_STYLE: Style = Style {
    bg: "#efe9db",
    floor: "#fbf7ec",
    line: "#3a3226",
    tree_line: "#3a3226",
    stone: "#e7e0cf",
    tree_shades: ["#efe9db", "#efe9db", "#efe9db"],
    shadow: "#8d8471",
    hatch: "#5a5342",
    tile: "#e7e0cf",
    mosaic: ["#f4edda", "#ede4cc", "#f0e8d3", "#e9dfc4"],
    pattern_line: "#3a3226",
    title: "#3a3226",
    water: "#a8c3cc",
    deep: "#7fa2b3",
    mud: "#e3d5b2",
};

const FOREST_STYLE: Style = Style {
    bg: "#2e4038",
    floor: "#c6c98e",
    line: "#2c3327",
    tree_line: "#2a3a2e",
    stone: "#b4b87e",
    tree_shades: ["#87a860", "#69894f", "#4d6942"],
    shadow: "#333c30",
    hatch: "#2e4038",
    tile: "#b7b4a2",
    mosaic: ["#c0c388", "#b8bb7f", "#c9cc93", "#b1b478"],
    pattern_line: "#2c3327",
    title: "#cfdcb4",
    water: "#a8c3cc",
    deep: "#7fa2b3",
    mud: "#adaf78",
};

const HEX_SIZE: f64 = 12.0;
const MARGIN: f64 = 16.0;

/// Waterline translucency: mud and pools let the floor (and any ruin tile
/// pattern) show through; deep water is markedly more opaque, and the
/// mud→pool→deep stacking compounds toward solid in the deepest basins.
const MUD_OPACITY: f64 = 0.55;
const WATER_OPACITY: f64 = 0.6;
const DEEP_OPACITY: f64 = 0.85;

/// Fixed-point coordinate displays: geometry is quantized at the source
/// (see `outline::quantize`), so these print with pure integer formatting —
/// far cheaper than the exact float-to-decimal path of `{:.1}`, and
/// byte-stable by construction. D0/D1/D2 print 0/1/2 decimal places.
struct D0(f64);
struct D1(f64);
struct D2(f64);

impl fmt::Display for D0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.round() as i64)
    }
}

impl fmt::Display for D1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = (self.0 * 10.0).round() as i64;
        let sign = if t < 0 { "-" } else { "" };
        write!(f, "{sign}{}.{}", t.abs() / 10, t.abs() % 10)
    }
}

impl fmt::Display for D2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = (self.0 * 100.0).round() as i64;
        let sign = if t < 0 { "-" } else { "" };
        write!(f, "{sign}{}.{:02}", t.abs() / 100, t.abs() % 100)
    }
}

const PALETTE: [&str; 12] = [
    "#e6194b", "#3cb44b", "#ffe119", "#4363d8", "#f58231", "#911eb4", "#42d4f4", "#f032e6",
    "#bfef45", "#fabed4", "#469990", "#dcbeff",
];

/// Render the finished map: parchment background, smoothed cave floor with a
/// dark wall outline (interior pillars via evenodd), hex grid clipped to the
/// floor.
pub fn svg(map: &CaveMap) -> String {
    svg_opts(map, false)
}

/// Like [`svg`], but with `labels` overlaying each area's index and a stable
/// content hash at its centroid — a diagnostic aid for pinpointing a specific
/// room across renders (see [`area_label_layer`]).
pub fn svg_opts(map: &CaveMap, labels: bool) -> String {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for lp in &map.outline {
        for &(x, y) in lp {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if map.outline.is_empty() {
        (min_x, min_y, max_x, max_y) = (0.0, 0.0, 100.0, 100.0);
    }
    let margin = 30.0;
    // Extra headroom so the title never overlaps the map.
    let title_band = 34.0;
    let vx = min_x - margin;
    let vy = min_y - margin - title_band;
    // Rough serif width estimate so long titles aren't clipped at the edge.
    let title_width = 16.0 + map.title.len() as f64 * 11.5;
    let vw = (max_x - min_x + 2.0 * margin).max(title_width + margin);
    let vh = max_y - min_y + 2.0 * margin + title_band;

    let (vxd, vyd, vwd, vhd) = (D1(vx), D1(vy), D1(vw), D1(vh));
    let (vw0, vh0) = (D0(vw), D0(vh));
    let floor_path = outline_path(&map.outline);
    let style = match map.mode {
        Mode::Cave => &CAVE_STYLE,
        Mode::Forest => &FOREST_STYLE,
    };

    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="{vxd} {vyd} {vwd} {vhd}" width="{vw0}" height="{vh0}">"##
    );
    let _ = write!(
        s,
        r##"<rect x="{vxd}" y="{vyd}" width="{vwd}" height="{vhd}" fill="{}"/>"##,
        style.bg
    );
    // The floor path is by far the largest string in the file and is used
    // five times (clip, mask, fill, shadow, border): define it once and
    // instantiate with <use>. Nonzero winding everywhere: overlapping area
    // loops (e.g. a projected ruin shape crossing a neighbour) merge into
    // one larger space instead of cancelling out, while pillar holes keep
    // their opposite winding.
    let _ = write!(s, r##"<defs><path id="fp" d="{floor_path}"/></defs>"##);
    let _ = write!(
        s,
        r##"<clipPath id="floor" clip-rule="nonzero"><use xlink:href="#fp" href="#fp"/></clipPath>"##
    );
    // Inverse of the floor: lets wall decorations (hatching, shadow, the
    // border itself) draw only on rock, so overlapping chambers lose the
    // barrier between them and bands never spill onto a floor.
    let _ = write!(
        s,
        r##"<mask id="rock" maskUnits="userSpaceOnUse" x="{vxd}" y="{vyd}" width="{vwd}" height="{vhd}"><rect x="{vxd}" y="{vyd}" width="{vwd}" height="{vhd}" fill="white"/><use xlink:href="#fp" href="#fp" fill="black" fill-rule="nonzero"/></mask>"##
    );

    // Border trees sit behind the floor fill: the clearing covers their
    // inner halves, leaving a jagged canopy ring around the edge. Deepest
    // band first so nearer (lighter) canopies overlap the darker ones.
    if !map.trees.is_empty() {
        for band in (0..style.tree_shades.len()).rev() {
            let _ = write!(
                s,
                r##"<g fill="{}" stroke="{}" stroke-width="1" stroke-linejoin="round">"##,
                style.tree_shades[band], style.tree_line
            );
            for (tree, depth) in &map.trees {
                if *depth != band {
                    continue;
                }
                let pts: Vec<String> =
                    tree.iter().map(|(x, y)| format!("{},{}", D1(*x), D1(*y))).collect();
                let _ = write!(s, r##"<polygon points="{}"/>"##, pts.join(" "));
            }
            s.push_str("</g>");
        }
    }

    // Floor fill only; the border stroke is drawn later, above the water,
    // so pools sit underneath the wall line and never thin it.
    let _ = write!(
        s,
        r##"<use xlink:href="#fp" href="#fp" fill="{}" fill-rule="nonzero"/>"##,
        style.floor
    );

    // Ruin floor tile pattern, directly on the floor so water floods over
    // it and the grid overlay stays legible above.
    if !map.floor_pattern.is_empty() {
        use crate::decor::PatternElem;
        let _ = write!(s, r##"<g clip-path="url(#floor)">"##);
        let mut curves = String::new();
        let mut elbows = String::new();
        for elem in &map.floor_pattern {
            match elem {
                PatternElem::Poly { pts, shade } => {
                    s.push_str(r##"<polygon points=""##);
                    for (i, (x, y)) in pts.iter().enumerate() {
                        if i > 0 {
                            s.push(' ');
                        }
                        let _ = write!(s, "{},{}", D1(*x), D1(*y));
                    }
                    let _ = write!(s, r##"" fill="{}"/>"##, style.mosaic[*shade as usize]);
                }
                PatternElem::Curve { from, ctrl, to } => {
                    let _ = write!(
                        curves,
                        "M{} {}Q{} {} {} {}",
                        D1(from.0), D1(from.1), D1(ctrl.0), D1(ctrl.1), D1(to.0), D1(to.1)
                    );
                }
                PatternElem::Elbow { from, tip, to } => {
                    let _ = write!(
                        elbows,
                        "M{} {}L{} {}L{} {}",
                        D1(from.0), D1(from.1), D1(tip.0), D1(tip.1), D1(to.0), D1(to.1)
                    );
                }
            }
        }
        if !curves.is_empty() {
            let _ = write!(
                s,
                r##"<path d="{curves}" fill="none" stroke="{}" stroke-width="1.6" stroke-opacity="0.22" stroke-linecap="round"/>"##,
                style.pattern_line
            );
        }
        if !elbows.is_empty() {
            let _ = write!(
                s,
                r##"<path d="{elbows}" fill="none" stroke="{}" stroke-width="1.0" stroke-opacity="0.28" stroke-linejoin="round"/>"##,
                style.pattern_line
            );
        }
        s.push_str("</g>");
    }

    // Waterline layers, lowest first: mud fringe under the pools, then the
    // pools, then the deep-water band inside them.
    // The waterline fills are translucent so ruin floor patterns show
    // through the shallows; deep water is markedly less transparent, and
    // the stacking (mud under pool under deep) compounds toward opaque.
    if !map.mud.is_empty() {
        let mud_path = outline_path(&map.mud);
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{mud_path}" fill="{}" fill-rule="evenodd" fill-opacity="{MUD_OPACITY}"/></g>"##,
            style.mud
        );
    }
    if !map.water.is_empty() {
        let water_path = outline_path(&map.water);
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{water_path}" fill="{}" fill-rule="evenodd" fill-opacity="{WATER_OPACITY}" stroke="{}" stroke-width="1.1" stroke-opacity="0.55"/></g>"##,
            style.water, style.line
        );
    }
    if !map.deep_water.is_empty() {
        let deep_path = outline_path(&map.deep_water);
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{deep_path}" fill="{}" fill-rule="evenodd" fill-opacity="{DEEP_OPACITY}"/></g>"##,
            style.deep
        );
    }

    // Grid overlay above the water, visible across the whole floor.
    match map.grid_style {
        GridStyle::Hex => {
            // One floor set up front: probing it is O(1) per cell, where
            // scanning doors/exit stubs per cell was O(cells x topology).
            let mut floor: HashSet<Hex> = HashSet::new();
            for area in &map.areas.cells {
                floor.extend(area.iter().copied());
            }
            floor.extend(map.topology.doors.iter().map(|d| d.cell));
            for e in &map.topology.exits {
                floor.extend(e.stub.iter().copied());
            }
            let near_floor =
                |h: Hex| floor.contains(&h) || h.neighbors().into_iter().any(|n| floor.contains(&n));
            let _ = write!(
                s,
                r##"<g clip-path="url(#floor)" stroke="{}" stroke-opacity="0.14" stroke-width="0.6" fill="none">"##,
                style.line
            );
            for &h in map.grid.cells() {
                if near_floor(h) {
                    let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
                }
            }
            s.push_str("</g>");
        }
        GridStyle::Square => {
            // Square cells at the hex column spacing (sqrt3 * size), so the
            // vertical lines meet the hex centres of every other row.
            let step = 3f64.sqrt() * HEX_SIZE;
            let mut d = String::new();
            let (k0, k1) = (
                (min_x / step).floor() as i64,
                (max_x / step).ceil() as i64,
            );
            for k in k0..=k1 {
                let x = (k as f64 + 0.5) * step;
                let _ = write!(d, "M{x} {mn}L{x} {mx}", x = D1(x), mn = D1(min_y), mx = D1(max_y));
            }
            let (m0, m1) = (
                (min_y / step).floor() as i64,
                (max_y / step).ceil() as i64,
            );
            for m in m0..=m1 {
                let y = (m as f64 + 0.5) * step;
                let _ = write!(d, "M{mn} {y}L{mx} {y}", y = D1(y), mn = D1(min_x), mx = D1(max_x));
            }
            let _ = write!(
                s,
                r##"<g clip-path="url(#floor)"><path d="{d}" stroke="{}" stroke-opacity="0.14" stroke-width="0.6" fill="none"/></g>"##,
                style.line
            );
        }
        GridStyle::None => {}
    }

    if !map.stones.is_empty() {
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)" fill="{}" stroke="{}" stroke-width="1" stroke-linejoin="round">"##,
            style.stone, style.line
        );
        for stone in &map.stones {
            let pts: Vec<String> = stone.iter().map(|(x, y)| format!("{},{}", D1(*x), D1(*y))).collect();
            let _ = write!(s, r##"<polygon points="{}"/>"##, pts.join(" "));
        }
        s.push_str("</g>");
    }

    // Each fan is an opaque object: its background-filled hull blanks out
    // whatever earlier fans it overlaps before its own strokes go down.
    if !map.hatching.is_empty() {
        let _ = write!(
            s,
            r##"<g mask="url(#rock)" stroke-linecap="round" stroke-width="1.1">"##
        );
        for fan in &map.hatching {
            let hull: Vec<String> = fan
                .hull
                .iter()
                .map(|(x, y)| format!("{},{}", D1(*x), D1(*y)))
                .collect();
            let mut d = String::new();
            for ((x1, y1), (x2, y2)) in &fan.strokes {
                let _ = write!(d, "M{} {}L{} {}", D1(*x1), D1(*y1), D1(*x2), D1(*y2));
            }
            let _ = write!(
                s,
                r##"<polygon points="{}" fill="{}" stroke="none"/><path d="{d}" fill="none" stroke="{}"/>"##,
                hull.join(" "),
                style.bg,
                style.hatch
            );
        }
        s.push_str("</g>");
    }

    // Faded stipple dots along ruin walls (cave mode), same layer slot as
    // the fans they replace.
    if !map.dots.is_empty() {
        let _ = write!(s, r##"<g mask="url(#rock)" fill="{}">"##, style.hatch);
        for &((x, y), r, a) in &map.dots {
            let _ = write!(
                s,
                r##"<circle cx="{cx}" cy="{cy}" r="{rr}" fill-opacity="{aa}"/>"##,
                cx = D1(x),
                cy = D1(y),
                rr = D2(r),
                aa = D2(a)
            );
        }
        s.push_str("</g>");
    }

    // The border's drop shadow: the border geometry itself, stroked,
    // translucent, offset down-right, and clipped to the floor — it falls
    // only on room contents (grid, water, tilework), never on the rock-side
    // decoration, so the outside patterns stay clean and the border reads
    // as floating above the rooms.
    let _ = write!(
        s,
        r##"<g clip-path="url(#floor)"><use xlink:href="#fp" href="#fp" transform="translate(1.5 2)" fill="none" stroke="{}" stroke-width="4.5" stroke-linejoin="round" stroke-opacity="0.7"/></g>"##,
        style.shadow
    );

    // Masonry tiles along ruin walls (forest mode), under the wall line so
    // their inner edge seats cleanly against it; masked to rock so a block
    // can never land inside a clearing.
    if !map.tiles.is_empty() {
        let _ = write!(
            s,
            r##"<g mask="url(#rock)" fill="{}" stroke="{}" stroke-width="0.45" stroke-linejoin="round">"##,
            style.tile, style.tree_line
        );
        for t in &map.tiles {
            let pts: Vec<String> = t.iter().map(|(x, y)| format!("{},{}", D1(*x), D1(*y))).collect();
            let _ = write!(s, r##"<polygon points="{}"/>"##, pts.join(" "));
        }
        s.push_str("</g>");
    }

    // Dungeon wall: a HOLLOW wall drawn as two thin lines, each the weight of
    // the organic border. The outer face is the traced boundary (stroked by
    // the border pass below); here we add the INNER face — each run offset one
    // band-width into its room. The gap between the two lines stays floor, so
    // the wall reads as a clean double line at organic thickness (no abrupt
    // thick→thin step where it meets a weathered wall) and the decor pass can
    // later fill the hollow with any masonry/pattern. Exactly the runs the
    // outline traced — never a full shape perimeter, which would stroke across
    // an overlapping neighbour's floor. Gaps between runs (doorway and exit
    // openings) stay unstroked.
    if !map.dungeon_walls.is_empty() {
        let w = DUNGEON_WALL_W * HEX_SIZE;
        let mut wall_d = String::new(); // hollow-wall outline (capsule per run)
        let mut shadow_d = String::new(); // inner line only (shadowed)
        // Emit a polyline as SVG path commands: `M` on the first point (unless
        // `continue_path`, which keeps drawing `L` from the current pen — used
        // to trace the capsule's inner side back onto its outer side).
        let emit = |d: &mut String, pts: &[Point], continue_path: bool| {
            for (i, &(x, y)) in pts.iter().enumerate() {
                let cmd = if i == 0 && !continue_path { "M" } else { "L" };
                let _ = write!(d, "{cmd}{} {}", D1(x), D1(y));
            }
        };
        for (shape, run) in &map.dungeon_walls {
            let inner = shape.shrink(w);
            let closed = run.len() > 2 && run.first() == run.last();
            let pts = if closed { &run[..run.len() - 1] } else { &run[..] };
            if pts.is_empty() {
                continue;
            }
            let inner_pts: Vec<Point> = pts.iter().map(|&p| inner.project(p)).collect();
            // Shadow follows the inner line only (closed only when the run is a
            // full ring; an open run's shadow stays an open polyline).
            emit(&mut shadow_d, &inner_pts, false);
            if closed {
                shadow_d.push('Z');
                // A room with no gaps: outer is the border, so stroke only the
                // inner ring.
                emit(&mut wall_d, &inner_pts, false);
            } else {
                // A run bounded by door/exit gaps: trace it as a closed
                // capsule — outer boundary forward, then the inner line back —
                // so the two jamb faces closing the gaps are mitred corners of
                // the outline, not centre-line stubs. This accounts for the
                // stroke thickness (no half-width poke into the opening) and
                // keeps the faces flush with the wall-line ends.
                emit(&mut wall_d, pts, false);
                let rev: Vec<Point> = inner_pts.iter().rev().copied().collect();
                emit(&mut wall_d, &rev, true);
            }
            wall_d.push('Z');
        }
        // Drop shadow of the inner lines only, offset into the room and
        // clipped to the floor, matching the border's own shadow depth.
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{shadow_d}" transform="translate(1.5 2)" fill="none" stroke="{}" stroke-width="4.5" stroke-linejoin="round" stroke-opacity="0.7"/></g>"##,
            style.shadow
        );
        let _ = write!(
            s,
            r##"<path d="{wall_d}" fill="none" stroke="{}" stroke-width="2.4" stroke-linejoin="round"/>"##,
            style.line
        );
    }

    // Dungeon doors, drawn UNDER the wall border so the jamb caps merge into
    // the wall line: a hex-aligned bar across the doorway with a dark cap at
    // each end. Wood / metal (banded) / portcullis (barred).
    s.push_str(&door_layer(map, style));

    // The wall border: cell-level unions guarantee simple loops with no
    // interior segments, so a plain full-weight stroke on top is correct.
    let _ = write!(
        s,
        r##"<use xlink:href="#fp" href="#fp" fill="none" stroke="{}" stroke-width="2.4" stroke-linejoin="round"/>"##,
        style.line
    );

    let _ = write!(
        s,
        r##"<text x="{}" y="{}" fill="{}" font-family="Georgia, serif" font-size="22" font-style="italic">{}</text>"##,
        D1(vx + 16.0),
        D1(vy + 34.0),
        style.title,
        map.title,
    );
    if labels {
        s.push_str(&area_label_layer(map));
    }
    s.push_str("</svg>");
    s
}

/// Overlay a label at every area's centroid and at each exit's outer end: a
/// **1-based** number for human readability, a kind letter (O/R/D for areas, E
/// for exits), and a stable 4-char content hash. The hash is derived from the
/// cell set alone, so the same physical feature prints the same code regardless
/// of how areas/exits happened to be ordered in a given build — letting two
/// renders (e.g. a CLI dump and the web demo) be cross-referenced even if their
/// indices diverge.
fn area_label_layer(map: &CaveMap) -> String {
    let mut s = String::new();
    // Drawn on top of the finished map, so a soft parchment halo keeps the
    // text legible over floor, water and hatching alike.
    let _ = write!(
        s,
        r##"<g font-family="Menlo, Consolas, monospace" text-anchor="middle" paint-order="stroke" stroke="{}" stroke-width="3.5" stroke-linejoin="round">"##,
        CAVE_STYLE.floor
    );
    let label = |cx: f64, cy: f64, tag: &str, hash: &str, sc: &mut String| {
        let _ = write!(
            sc,
            r##"<text x="{}" y="{}" font-size="15" font-weight="bold" fill="#b4231f">{tag}</text>"##,
            D1(cx),
            D1(cy - 1.0),
        );
        let _ = write!(
            sc,
            r##"<text x="{}" y="{}" font-size="9" fill="#3a3226">{hash}</text>"##,
            D1(cx),
            D1(cy + 11.0),
        );
    };
    for i in 0..map.areas.count() {
        let cells = &map.areas.cells[i];
        if cells.is_empty() {
            continue;
        }
        let (mut cx, mut cy) = (0.0, 0.0);
        for &c in cells {
            let (x, y) = c.center(HEX_SIZE);
            cx += x;
            cy += y;
        }
        let n = cells.len() as f64;
        let kind = match map.areas.kind(i) {
            AreaKind::Organic => 'O',
            AreaKind::Ruin => 'R',
            AreaKind::Dungeon => 'D',
        };
        label(cx / n, cy / n, &format!("{}{kind}", i + 1), &area_hash(&map.areas.cells[i]), &mut s);
    }
    // Exits: label the outer end (last stub cell) so exit passages can be named
    // like rooms (1E, 2E, ...).
    for (k, e) in map.topology.exits.iter().enumerate() {
        if let Some(&last) = e.stub.last() {
            let (cx, cy) = last.center(HEX_SIZE);
            label(cx, cy, &format!("{}E", k + 1), &area_hash(&[last]), &mut s);
        }
    }
    s.push_str("</g>");
    s
}

/// A stable 4-char base-36 code for an area, hashed (FNV-1a) from its sorted
/// cell coordinates. Depends only on the cells, never on the area's index.
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

/// √3/2 — the apothem of a unit-side hex (centre to edge midpoint).
const HEX_APOTHEM: f64 = crate::doorway::HEX_APOTHEM;
/// Half-width of a door leaf, across the bar, in hex-size units.
const DOOR_LEAF_HALF: f64 = 0.26;
/// Radius of a jamb cap (the dark stub anchoring a door into the wall).
const DOOR_CAP_R: f64 = 0.30;
/// Radius of one portcullis bar (drawn as a ring).
const DOOR_BAR_R: f64 = 0.14;
/// Thickness of the dungeon wall band, in hex-size units — a shade wider
/// than a door leaf (2·[`DOOR_LEAF_HALF`] plus its stroke), so a flush bar
/// sits fully within the wall's thickness.
const DUNGEON_WALL_W: f64 = 0.6;

/// The doors layer: for every mouth onto a dungeon room (see
/// [`crate::doorway`]) a bar spanning the opening with a dark jamb cap at
/// each end, drawn **flush on the wall line** — the bar sits within the
/// thick dungeon wall band, so the door has no visible depth on either
/// side. A multi-door mouth gets one **double door** spanning the whole
/// opening: a longer bar with a seam per leaf (or a longer portcullis).
/// Drawn as one group so the caller can slip it *under* the wall border,
/// where the caps merge into the wall line. Returns `""` when the map has
/// no dungeon doors.
fn door_layer(map: &CaveMap, style: &Style) -> String {
    let ap = HEX_SIZE * HEX_APOTHEM; // half a single doorway's span
    let lw = DOOR_LEAF_HALF * HEX_SIZE;
    let cr = DOOR_CAP_R * HEX_SIZE;
    let pr = DOOR_BAR_R * HEX_SIZE;
    let mut caps = String::new(); // jamb caps (wall colour, filled)
    let mut leaves = String::new(); // wood/metal leaf rectangles (floor-filled)
    let mut marks = String::new(); // metal bands + leaf seams (stroke only)
    let mut bars = String::new(); // portcullis bars (rings)

    let doors = &map.topology.doors;
    for m in &map.mouths {
        // Wall-anchored bars sit half the wall band INSIDE the room, so the
        // leaf spans the inward-thick wall: its outer face flush with the
        // traced boundary, no visible depth on the inside.
        let inward = match m.anchor {
            crate::doorway::Anchor::Wall => 0.5 * DUNGEON_WALL_W * HEX_SIZE,
            crate::doorway::Anchor::Midgap | crate::doorway::Anchor::Free => 0.0,
        };
        let (t0, t1) = (-m.opening / 2.0, m.opening / 2.0);
        // The opening's two endpoints, pulled `inward` into the room. On a
        // round room these sit on the ring at the ±half-opening arc, so the
        // straight bar drawn between them is a chord — a touch narrower than
        // the arc, and wholly inside the ring (never poking out as a wide
        // tangent bar would).
        let endpoint = |t: f64| -> Point {
            if let Some(sh @ crate::ruins::RuinShape::Circle { cx, cy, .. }) = m.shape {
                let wp = sh.wall_point(sh.wall_param(m.center) + t);
                let (nx, ny) = (cx - wp.0, cy - wp.1);
                let d = nx.hypot(ny).max(1e-9);
                (wp.0 + nx / d * inward, wp.1 + ny / d * inward)
            } else {
                (m.center.0 + m.axis.0 * t - m.out.0 * inward,
                 m.center.1 + m.axis.1 * t - m.out.1 * inward)
            }
        };
        let (e1, e2) = (endpoint(t0), endpoint(t1));
        // The bar itself stays perfectly straight: interpolate along the
        // chord between the two endpoints.
        let at = |t: f64| -> Point {
            let f = (t - t0) / (t1 - t0);
            (e1.0 + (e2.0 - e1.0) * f, e1.1 + (e2.1 - e1.1) * f)
        };
        // One glyph for the whole mouth; a double door takes the "strongest"
        // member style, so a portcullis half never downgrades to plain wood
        // and a doored half never loses its leaf to an `Open` sibling.
        let rank = |s: DoorStyle| match s {
            DoorStyle::Open => 0,
            DoorStyle::Wood => 1,
            DoorStyle::Metal => 2,
            DoorStyle::Portcullis => 3,
        };
        let kind = m
            .members
            .iter()
            .filter(|&&i| map.is_dungeon(doors[i].a) || map.is_dungeon(doors[i].b))
            .map(|&i| map.door_styles[i])
            .max_by_key(|&s| rank(s))
            .unwrap_or_default();
        if kind == DoorStyle::Open {
            // An open doorway: the framed gap stands on its own — no bar, no
            // caps.
            continue;
        }
        let _ = write!(
            caps,
            r##"<circle cx="{}" cy="{}" r="{cr}"/><circle cx="{}" cy="{}" r="{cr}"/>"##,
            D1(e1.0), D1(e1.1), D1(e2.0), D1(e2.1), cr = D1(cr),
        );
        // Leaf normal from the chord between the ends (the leaf plank spans
        // the opening; on a circle its ends sit on the ring, its centre a
        // touch inside — reads as recessed, never protruding).
        let (dx, dy) = (e2.0 - e1.0, e2.1 - e1.1);
        let dl = dx.hypot(dy).max(1e-9);
        let perp = (-dy / dl, dx / dl);
        // One leaf per hex of opening (a double/triple door for wide mouths).
        let leaves_n = (m.opening / (2.0 * ap)).round().max(1.0) as usize;
        match kind {
            DoorStyle::Portcullis => {
                // Five bars per leaf, evenly spread across the mouth.
                let n = 5 * leaves_n;
                let (b0, b1) = (t0 + 0.33 * ap, t1 - 0.33 * ap);
                for k in 0..n {
                    let t = b0 + (b1 - b0) * (k as f64 + 0.5) / n as f64;
                    let p = at(t);
                    let _ = write!(bars, r##"<circle cx="{}" cy="{}" r="{}"/>"##, D1(p.0), D1(p.1), D1(pr));
                }
            }
            kind => {
                // A leaf rectangle spanning the mouth.
                let corner = |t: f64, sp: f64| {
                    let p = at(t);
                    (p.0 + perp.0 * lw * sp, p.1 + perp.1 * lw * sp)
                };
                let (p1, p2, p3, p4) =
                    (corner(t1, 1.0), corner(t0, 1.0), corner(t0, -1.0), corner(t1, -1.0));
                let _ = write!(
                    leaves,
                    "M{} {}L{} {}L{} {}L{} {}Z",
                    D1(p1.0), D1(p1.1), D1(p2.0), D1(p2.1), D1(p3.0), D1(p3.1), D1(p4.0), D1(p4.1),
                );
                // Double doors: a seam between each pair of leaves.
                for k in 1..leaves_n {
                    let mid = t0 + m.opening * k as f64 / leaves_n as f64;
                    let (a, b) = (corner(mid, 1.0), corner(mid, -1.0));
                    let _ = write!(marks, "M{} {}L{} {}", D1(a.0), D1(a.1), D1(b.0), D1(b.1));
                }
                if kind == DoorStyle::Metal {
                    // A band down the whole bar.
                    let (a, b) = (at(t0 + 0.28 * ap), at(t1 - 0.28 * ap));
                    let _ = write!(marks, "M{} {}L{} {}", D1(a.0), D1(a.1), D1(b.0), D1(b.1));
                }
            }
        }
    }
    if caps.is_empty() {
        return String::new();
    }
    // Caps under the leaves so only their rounded ends show beyond the bar.
    // No floor clip: a flush bar straddles the wall line, half of it over
    // the rock-side half of the wall band.
    let mut out = String::new();
    let _ = write!(out, r##"<g><g fill="{}">{caps}</g>"##, style.line);
    if !leaves.is_empty() {
        let _ = write!(
            out,
            r##"<path d="{leaves}" fill="{}" stroke="{}" stroke-width="1.6" stroke-linejoin="round"/>"##,
            style.floor, style.line
        );
    }
    if !bars.is_empty() {
        let _ = write!(
            out,
            r##"<g fill="{}" stroke="{}" stroke-width="1.4">{bars}</g>"##,
            style.floor, style.line
        );
    }
    if !marks.is_empty() {
        let _ = write!(
            out,
            r##"<path d="{marks}" fill="none" stroke="{}" stroke-width="1.3" stroke-linecap="round"/>"##,
            style.line
        );
    }
    out.push_str("</g>");
    out
}


fn outline_path(loops: &[Vec<Point>]) -> String {
    let mut d = String::new();
    for lp in loops {
        for (i, &(x, y)) in lp.iter().enumerate() {
            let cmd = if i == 0 { "M" } else { "L" };
            let _ = write!(d, "{cmd}{} {}", D1(x), D1(y));
        }
        d.push('Z');
    }
    d
}


pub fn debug_svg(map: &CaveMap) -> String {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &h in map.grid.cells() {
        for (x, y) in h.corners(HEX_SIZE) {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let vx = min_x - MARGIN;
    let vy = min_y - MARGIN;
    let vw = max_x - min_x + 2.0 * MARGIN;
    let vh = max_y - min_y + 2.0 * MARGIN;
    let (vxd, vyd, vwd, vhd) = (D1(vx), D1(vy), D1(vw), D1(vh));
    let (vw0, vh0) = (D0(vw), D0(vh));

    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vxd} {vyd} {vwd} {vhd}" width="{vw0}" height="{vh0}">"##
    );
    let _ = write!(
        s,
        r##"<rect x="{vxd}" y="{vyd}" width="{vwd}" height="{vhd}" fill="#16161e"/>"##
    );

    s.push_str(r##"<g stroke="#2c2c38" stroke-width="0.5" fill="none">"##);
    for &h in map.grid.cells() {
        let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
    }
    s.push_str("</g>");

    for (i, area) in map.areas.cells.iter().enumerate() {
        let color = PALETTE[i % PALETTE.len()];
        let _ = write!(s, r##"<g fill="{color}" fill-opacity="0.85" stroke="none">"##);
        for &h in area {
            let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
        }
        s.push_str("</g>");
    }

    // Dungeon areas: outline their cells in cyan so the ruin/dungeon split is
    // visible over the per-area fills.
    s.push_str(r##"<g fill="none" stroke="#31d2f2" stroke-width="1.4">"##);
    for (i, area) in map.areas.cells.iter().enumerate() {
        if !map.is_dungeon(i) {
            continue;
        }
        for &h in area {
            let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
        }
    }
    s.push_str("</g>");

    s.push_str(r##"<g fill="#f2f2ea" stroke="none">"##);
    for d in &map.topology.doors {
        let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(d.cell));
    }
    s.push_str("</g>");

    s.push_str(r##"<g fill="#ff8c42" stroke="none">"##);
    for e in &map.topology.exits {
        for &h in &e.stub {
            let _ = write!(s, r##"<polygon points="{}"/>"##, hex_points(h));
        }
    }
    s.push_str("</g>");

    let n_corridors = map.topology.is_corridor.iter().filter(|&&c| c).count();
    let n_ruin = map.areas.kinds().iter().filter(|&&k| k == AreaKind::Ruin).count();
    let n_dungeon = map.areas.kinds().iter().filter(|&&k| k == AreaKind::Dungeon).count();
    let _ = write!(
        s,
        r##"<text x="{}" y="{}" fill="#aaaab4" font-family="monospace" font-size="11">seed {} | tags: {} | {} areas ({} ruin, {} dungeon), {} doors, {} corridors, {} exits</text>"##,
        D1(vx + 6.0),
        D1(vy + 14.0),
        map.seed,
        map.tags,
        map.areas.count(),
        n_ruin,
        n_dungeon,
        map.topology.doors.len(),
        n_corridors,
        map.topology.exits.len(),
    );

    s.push_str("</svg>");
    s
}

fn hex_points(h: Hex) -> String {
    h.corners(HEX_SIZE)
        .iter()
        .map(|(x, y)| format!("{},{}", D2(*x), D2(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}
