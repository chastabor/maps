//! SVG renderers: `svg` draws the finished map from the smoothed outline;
//! `debug_svg` shows the raw hex cells, one colour per area.

use crate::grid::Hex;
use crate::outline::Point;
use crate::{CaveMap, GridStyle, Mode};
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
    mosaic: [&'static str; 4],
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
    shadow: "#2e4038",
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

const PALETTE: [&str; 12] = [
    "#e6194b", "#3cb44b", "#ffe119", "#4363d8", "#f58231", "#911eb4", "#42d4f4", "#f032e6",
    "#bfef45", "#fabed4", "#469990", "#dcbeff",
];

/// Render the finished map: parchment background, smoothed cave floor with a
/// dark wall outline (interior pillars via evenodd), hex grid clipped to the
/// floor.
pub fn svg(map: &CaveMap) -> String {
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

    let floor_path = outline_path(&map.outline);
    let style = match map.mode {
        Mode::Cave => &CAVE_STYLE,
        Mode::Forest => &FOREST_STYLE,
    };

    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vx:.1} {vy:.1} {vw:.1} {vh:.1}" width="{vw:.0}" height="{vh:.0}">"##
    );
    let _ = write!(
        s,
        r##"<rect x="{vx:.1}" y="{vy:.1}" width="{vw:.1}" height="{vh:.1}" fill="{}"/>"##,
        style.bg
    );
    // Nonzero winding everywhere: overlapping area loops (e.g. a projected
    // ruin shape crossing a neighbour) merge into one larger space instead
    // of cancelling out, while pillar holes keep their opposite winding.
    let _ = write!(
        s,
        r##"<clipPath id="floor" clip-rule="nonzero"><path d="{floor_path}"/></clipPath>"##
    );
    // Inverse of the floor: lets wall decorations (hatching, shadow, the
    // border itself) draw only on rock, so overlapping chambers lose the
    // barrier between them and bands never spill onto a floor.
    let _ = write!(
        s,
        r##"<mask id="rock" maskUnits="userSpaceOnUse" x="{vx:.1}" y="{vy:.1}" width="{vw:.1}" height="{vh:.1}"><rect x="{vx:.1}" y="{vy:.1}" width="{vw:.1}" height="{vh:.1}" fill="white"/><path d="{floor_path}" fill="black" fill-rule="nonzero"/></mask>"##
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
                    tree.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
                let _ = write!(s, r##"<polygon points="{}"/>"##, pts.join(" "));
            }
            s.push_str("</g>");
        }
    }

    // Floor fill only; the border stroke is drawn later, above the water,
    // so pools sit underneath the wall line and never thin it.
    let _ = write!(
        s,
        r##"<path d="{floor_path}" fill="{}" fill-rule="nonzero"/>"##,
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
                    let p: Vec<String> =
                        pts.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
                    let _ = write!(
                        s,
                        r##"<polygon points="{}" fill="{}"/>"##,
                        p.join(" "),
                        style.mosaic[*shade as usize % 4]
                    );
                }
                PatternElem::Curve { from, ctrl, to } => {
                    let _ = write!(
                        curves,
                        "M{:.1} {:.1}Q{:.1} {:.1} {:.1} {:.1}",
                        from.0, from.1, ctrl.0, ctrl.1, to.0, to.1
                    );
                }
                PatternElem::Elbow { from, tip, to } => {
                    let _ = write!(
                        elbows,
                        "M{:.1} {:.1}L{:.1} {:.1}L{:.1} {:.1}",
                        from.0, from.1, tip.0, tip.1, to.0, to.1
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
    if !map.mud.is_empty() {
        let mud_path = outline_path(&map.mud);
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{mud_path}" fill="{}" fill-rule="evenodd"/></g>"##,
            style.mud
        );
    }
    if !map.water.is_empty() {
        let water_path = outline_path(&map.water);
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{water_path}" fill="{}" fill-rule="evenodd" stroke="{}" stroke-width="1.1" stroke-opacity="0.55"/></g>"##,
            style.water, style.line
        );
    }
    if !map.deep_water.is_empty() {
        let deep_path = outline_path(&map.deep_water);
        let _ = write!(
            s,
            r##"<g clip-path="url(#floor)"><path d="{deep_path}" fill="{}" fill-rule="evenodd"/></g>"##,
            style.deep
        );
    }

    // Grid overlay above the water, visible across the whole floor.
    match map.grid_style {
        GridStyle::Hex => {
            let _ = write!(
                s,
                r##"<g clip-path="url(#floor)" stroke="{}" stroke-opacity="0.14" stroke-width="0.6" fill="none">"##,
                style.line
            );
            for &h in map.grid.cells() {
                if near_floor(map, h) {
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
                let _ = write!(d, "M{x:.1} {min_y:.1}L{x:.1} {max_y:.1}");
            }
            let (m0, m1) = (
                (min_y / step).floor() as i64,
                (max_y / step).ceil() as i64,
            );
            for m in m0..=m1 {
                let y = (m as f64 + 0.5) * step;
                let _ = write!(d, "M{min_x:.1} {y:.1}L{max_x:.1} {y:.1}");
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
            let pts: Vec<String> = stone.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
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
                .map(|(x, y)| format!("{x:.1},{y:.1}"))
                .collect();
            let mut d = String::new();
            for ((x1, y1), (x2, y2)) in &fan.strokes {
                let _ = write!(d, "M{x1:.1} {y1:.1}L{x2:.1} {y2:.1}");
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
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="{r:.2}" fill-opacity="{a:.2}"/>"##
            );
        }
        s.push_str("</g>");
    }

    // The shadow band sits above the hatching so fans never blank it out,
    // but at partial opacity so their strokes still read through; masked to
    // rock so only the outer half of the stroke shows.
    let _ = write!(
        s,
        r##"<g mask="url(#rock)"><path d="{floor_path}" fill="none" stroke="{}" stroke-width="9" stroke-linejoin="round" stroke-opacity="0.7"/></g>"##,
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
            let pts: Vec<String> = t.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
            let _ = write!(s, r##"<polygon points="{}"/>"##, pts.join(" "));
        }
        s.push_str("</g>");
    }

    // The wall border: cell-level unions guarantee simple loops with no
    // interior segments, so a plain full-weight stroke on top is correct.
    let _ = write!(
        s,
        r##"<path d="{floor_path}" fill="none" stroke="{}" stroke-width="2.4" stroke-linejoin="round"/>"##,
        style.line
    );

    let _ = write!(
        s,
        r##"<text x="{:.1}" y="{:.1}" fill="{}" font-family="Georgia, serif" font-size="22" font-style="italic">{}</text>"##,
        vx + 16.0,
        vy + 34.0,
        style.title,
        map.title,
    );
    s.push_str("</svg>");
    s
}

fn outline_path(loops: &[Vec<Point>]) -> String {
    let mut d = String::new();
    for lp in loops {
        for (i, &(x, y)) in lp.iter().enumerate() {
            let cmd = if i == 0 { "M" } else { "L" };
            let _ = write!(d, "{cmd}{x:.1} {y:.1}");
        }
        d.push('Z');
    }
    d
}

/// True if the cell or any neighbour is cave floor (limits the grid overlay
/// to hexes that can actually show through the clip).
fn near_floor(map: &CaveMap, h: Hex) -> bool {
    let is_floor = |c: Hex| {
        map.areas.owner_of(c).is_some()
            || map.topology.doors.iter().any(|d| d.cell == c)
            || map.topology.exits.iter().any(|e| e.stub.contains(&c))
    };
    is_floor(h) || h.neighbors().into_iter().any(is_floor)
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

    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vx:.1} {vy:.1} {vw:.1} {vh:.1}" width="{vw:.0}" height="{vh:.0}">"##
    );
    let _ = write!(
        s,
        r##"<rect x="{vx:.1}" y="{vy:.1}" width="{vw:.1}" height="{vh:.1}" fill="#16161e"/>"##
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
    let _ = write!(
        s,
        r##"<text x="{:.1}" y="{:.1}" fill="#aaaab4" font-family="monospace" font-size="11">seed {} | tags: {} | {} areas, {} doors, {} corridors, {} exits</text>"##,
        vx + 6.0,
        vy + 14.0,
        map.seed,
        map.tags,
        map.areas.count(),
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
        .map(|(x, y)| format!("{x:.2},{y:.2}"))
        .collect::<Vec<_>>()
        .join(" ")
}
