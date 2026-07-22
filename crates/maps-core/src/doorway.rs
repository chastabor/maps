//! Doorway mouths onto dungeon rooms.
//!
//! Doors whose cells are hex-adjacent carve one merged opening in the wall,
//! so doors are first clustered (union-find on cell adjacency) and each
//! dungeon-touching cluster becomes a [`Mouth`]: where the opening pierces
//! the room wall, which way it runs, and how wide it spans. Doorways are
//! **flush**: the opening is a gap cut into the room's exact wall (clamped so
//! it never crosses a rect corner — see [`clamp_opening`]) and nothing is
//! built outside the measured shape. Two consumers share the mouths so they
//! always agree:
//!
//! * the outline pipeline snaps its wall-splice endpoints to the mouth's
//!   [`Jamb`]s, cutting the gap to the controlled opening width;
//! * the renderer draws the door glyph (bar + jamb caps) flush on the wall
//!   line, with the thick dungeon wall band hiding the organic corridor's
//!   seam.

use crate::AreaKind;
use crate::grid::Hex;
use crate::growth::Areas;
use crate::outline::Point;
use crate::ruins::RuinShape;
use crate::topology::{Door, Exit, Topology};
use std::collections::{HashMap, HashSet};

/// √3/2 — the apothem of a unit-side hex (centre to edge midpoint). Half of
/// a single doorway's span, in hex-size units.
pub const HEX_APOTHEM: f64 = crate::grid::SQRT3 / 2.0;

/// The three across-flats hex axes (edge-midpoint to opposite edge-midpoint),
/// as unit vectors at 0°, 60° and 120°. A mouth with no usable wall geometry
/// snaps to whichever its passage runs most nearly across.
const DOOR_AXES: [(f64, f64); 3] = [(1.0, 0.0), (0.5, HEX_APOTHEM), (-0.5, HEX_APOTHEM)];

/// How a mouth's centre point was anchored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// On a single dungeon room's wall; the lip extends outward from it.
    Wall,
    /// Midway between two dungeon rooms' walls (a door piercing both).
    Midgap,
    /// No usable wall geometry; the centre is the cluster's cell centroid.
    Free,
}

/// One opening onto a dungeon room: a cluster of hex-adjacent doors and the
/// geometry of the gap they carve in the room wall.
#[derive(Clone, Debug)]
pub struct Mouth {
    /// Indices into `topology.doors`, ascending.
    pub members: Vec<usize>,
    pub anchor: Anchor,
    /// The opening's centre: on the anchor room's wall, slid along its edge
    /// so the full opening fits (`Wall`); midway between the two facing
    /// walls (`Midgap`); or the cluster's cell centroid (`Free`). The wall
    /// gap and the door bar are both centred here.
    pub center: Point,
    /// Unit direction out of the room, through the doorway.
    pub out: Point,
    /// Unit wall tangent — the direction the door bar runs.
    pub axis: Point,
    /// The anchor room's shape for a `Wall` mouth (`None` for `Midgap`/`Free`,
    /// whose wall is straight): lets the renderer bend a wide bar along a
    /// circle's arc so a double door on a round room stays inside the ring.
    pub shape: Option<RuinShape>,
    /// Controlled full width of the opening cut into the wall: one hex per
    /// member door (capped), never less than one hex — the door bar and the
    /// wall gap take this same size, so a bar always closes its opening and
    /// an opening never pinches below its doorway.
    pub opening: f64,
}

/// Cluster the map's doors into mouths. Only clusters that touch a dungeon
/// room produce one — other doors carve plain organic gaps and draw nothing.
pub fn mouths(topology: &Topology, areas: &Areas, s: f64) -> Vec<Mouth> {
    let find = crate::growth::find;
    let doors = &topology.doors;
    let dungeon = |i: usize| areas.kind(i) == AreaKind::Dungeon;
    let mut root: Vec<usize> = (0..doors.len()).collect();
    // Two doors merge into one mouth only when they carve one opening in one
    // room's wall: cells hex-adjacent AND sharing a dungeon room. (Adjacent
    // cells serving unrelated rooms pierce different walls — merging them
    // would hang one long door bar across open floor between the openings.)
    let shared_room = |a: &Door, b: &Door| {
        [a.a, a.b]
            .into_iter()
            .filter(|&r| dungeon(r))
            .any(|r| r == b.a || r == b.b)
    };
    for i in 0..doors.len() {
        for j in i + 1..doors.len() {
            if doors[i].cell.distance(doors[j].cell) <= 1 && shared_room(&doors[i], &doors[j]) {
                let (a, b) = (find(&mut root, i), find(&mut root, j));
                root[a] = b;
            }
        }
    }
    let mut clusters: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..doors.len() {
        let r = find(&mut root, i);
        clusters.entry(r).or_default().push(i);
    }
    clusters
        .into_values()
        .filter(|members| members.iter().any(|&i| dungeon(doors[i].a) || dungeon(doors[i].b)))
        .filter_map(|members| mouth(members, doors, areas, s))
        .collect()
}

fn mouth(members: Vec<usize>, doors: &[Door], areas: &Areas, s: f64) -> Option<Mouth> {
    let dungeon = |i: usize| areas.kind(i) == AreaKind::Dungeon;
    let centers: Vec<Point> = members.iter().map(|&i| doors[i].cell.center(s)).collect();
    let c0 = centers.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let c0 = (c0.0 / centers.len() as f64, c0.1 / centers.len() as f64);
    let mut rooms: Vec<usize> = members
        .iter()
        .flat_map(|&i| [doors[i].a, doors[i].b])
        .filter(|&r| dungeon(r))
        .collect();
    rooms.sort_unstable();
    rooms.dedup();
    let shapes = areas.shapes();

    // The mean passage direction across the mouth, oriented away from a
    // dungeon room. Picks which wall a rectangle's mouth pierces (a corner-
    // adjacent door cell sits diagonally off the room, so position alone
    // misclassifies the wall) and orients free mouths.
    let travel = {
        let mut acc = (0.0, 0.0);
        for &i in &members {
            if let Some(u) = passage_dir(&doors[i], areas, s) {
                // u runs a→b; flip so it always points away from a room.
                let t = if dungeon(doors[i].a) { u } else { (-u.0, -u.1) };
                acc = (acc.0 + t.0, acc.1 + t.1);
            }
        }
        let len = acc.0.hypot(acc.1);
        (len > 1e-6).then(|| (acc.0 / len, acc.1 / len))
    };

    // One room touched → anchored on its wall, the lip perpendicular to it.
    // Two rooms → **flush placement**: only when their pierced walls face
    // each other squarely (parallel, openings collinear) does the lip sit
    // centered between them; otherwise the door belongs to one room — the
    // one whose wall the passage hits most squarely — and sits flush in its
    // wall, never averaged askew across the gap. Otherwise (no shape /
    // degenerate) → free: the mean passage direction, or the nearest
    // across-flats hex axis.
    let ap = HEX_APOTHEM * s;
    let flush = |sh, (wall, out, axis)| (Anchor::Wall, wall, out, axis, Some(sh));
    let anchored = match rooms[..] {
        [r] => shapes[r].and_then(|sh| wall_anchor(sh, c0, travel).map(|w| flush(sh, w))),
        [ra, rb] => {
            let a = shapes[ra].and_then(|sh| wall_anchor(sh, c0, travel).map(|w| (sh, w)));
            let b = shapes[rb].and_then(|sh| wall_anchor(sh, c0, travel).map(|w| (sh, w)));
            match (a, b) {
                (Some((sa, (wa, oa, aa))), Some((sb, (wb, ob, ab)))) => {
                    let d = (wb.0 - wa.0, wb.1 - wa.1);
                    let len = d.0.hypot(d.1);
                    let parallel = (aa.0 * ab.0 + aa.1 * ab.1).abs() > 0.98;
                    let lateral = (d.0 * aa.0 + d.1 * aa.1).abs();
                    if parallel && lateral < 0.35 * ap && len > 1e-6 {
                        let out = (d.0 / len, d.1 / len);
                        let wall = ((wa.0 + wb.0) / 2.0, (wa.1 + wb.1) / 2.0);
                        Some((Anchor::Midgap, wall, out, (-out.1, out.0), None))
                    } else {
                        // Squarest wall wins; ties go to the lower room index.
                        let da = travel.map_or(0.0, |t| (t.0 * oa.0 + t.1 * oa.1).abs());
                        let db = travel.map_or(0.0, |t| (t.0 * ob.0 + t.1 * ob.1).abs());
                        Some(if da >= db { flush(sa, (wa, oa, aa)) } else { flush(sb, (wb, ob, ab)) })
                    }
                }
                (Some((sh, w)), None) | (None, Some((sh, w))) => Some(flush(sh, w)),
                _ => None,
            }
        }
        _ => None,
    };
    let (anchor, wall, out, axis, anchor_shape) = anchored.or_else(|| {
        if let Some(out) = travel {
            Some((Anchor::Free, c0, out, (-out.1, out.0), None))
        } else {
            // Degenerate mean → nearest across-flats hex axis to the lead.
            let u = passage_dir(&doors[members[0]], areas, s)?;
            let t = (-u.1, u.0);
            let axis = DOOR_AXES.into_iter().max_by(|a, b| {
                (t.0 * a.0 + t.1 * a.1).abs().total_cmp(&(t.0 * b.0 + t.1 * b.1).abs())
            })?;
            Some((Anchor::Free, c0, (-axis.1, axis.0), axis, None))
        }
    })?;

    // One hex of opening per member door, capped at a triple gate.
    let opening = members.len().min(3) as f64 * 2.0 * ap;
    // Centre the opening on the member cells' span along the wall, then slide
    // it within the pierced edge so the gap never crosses a corner.
    let (lo, hi) = centers
        .iter()
        .map(|p| (p.0 - wall.0) * axis.0 + (p.1 - wall.1) * axis.1)
        .fold((f64::MAX, f64::MIN), |(lo, hi), t| (lo.min(t), hi.max(t)));
    let mid = (lo + hi) / 2.0;
    let raw = (wall.0 + axis.0 * mid, wall.1 + axis.1 * mid);
    let center = match anchor_shape {
        Some(sh) => clamp_opening(sh, raw, opening / 2.0, out),
        None => raw,
    };
    Some(Mouth { members, anchor, center, out, axis, shape: anchor_shape, opening })
}

/// Slide an opening of half-width `half` along the one wall edge that the
/// outward normal `out` selects, so the whole opening fits within that edge —
/// a doorway gap may never cross a rect corner. Clamping is confined to the
/// `out` edge and never slides onto an adjacent one: the caller's `out`/`axis`
/// stay valid, so a bar can't end up drawn along the wrong wall. An edge
/// shorter than the opening centres it; a circle has no corners, so its point
/// just projects back onto the ring.
pub(crate) fn clamp_opening(shape: RuinShape, p: Point, half: f64, out: Point) -> Point {
    match shape {
        RuinShape::Rect { cx, cy, hw, hh } => {
            if out.0.abs() >= out.1.abs() {
                // Left/right edge: x pinned to the wall, slide along y.
                let sx = if out.0 >= 0.0 { 1.0 } else { -1.0 };
                let y = if hh <= half { cy } else { p.1.clamp(cy - hh + half, cy + hh - half) };
                (cx + sx * hw, y)
            } else {
                // Top/bottom edge: y pinned to the wall, slide along x.
                let sy = if out.1 >= 0.0 { 1.0 } else { -1.0 };
                let x = if hw <= half { cx } else { p.0.clamp(cx - hw + half, cx + hw - half) };
                (x, cy + sy * hh)
            }
        }
        RuinShape::Circle { .. } => shape.project(p),
        _ => p,
    }
}

/// A doorway jamb anchor for the outline's wall splice: the room's shape, the
/// opening centre projected onto that wall, and the half-opening width. The
/// splice snaps its run endpoints to `center ± half` along the wall, so the
/// gap cut into the exact wall matches the door bar's span.
pub struct Jamb {
    pub shape: RuinShape,
    pub center: Point,
    pub half: f64,
}

/// Insert every dungeon-exit plug into `ruin_map`, so the outline pipeline
/// projects those cells onto their straight throats. (Doorways build
/// nothing: a door is a flush gap in the room's exact wall, and the organic
/// corridor meets it directly.) Returns `(plug_cells, clean_shapes)`: all
/// plugged cells (excluded from the weathered ruin decor) and the wall
/// geometry that keeps the clean treatment — the dungeon rooms themselves
/// and a short **lip hall** at each exit mouth (an exit stub's farther walls
/// hatch like any organic passage). Decor classifies against these shapes
/// rather than cells: a cell lookup misses e.g. a rectangle's corners, which
/// no hex cell contains, and hatched them organic.
pub fn apply_plugs(
    ruin_map: &mut HashMap<Hex, RuinShape>,
    topology: &Topology,
    areas: &Areas,
    s: f64,
) -> (HashSet<Hex>, Vec<RuinShape>) {
    let mut plug_cells = HashSet::new();
    let mut clean_shapes: Vec<RuinShape> = (0..areas.count())
        .filter(|&i| areas.kind(i) == AreaKind::Dungeon)
        .filter_map(|i| areas.shapes()[i])
        .collect();
    for e in &topology.exits {
        if let Some((full, lip)) = exit_plug(e, areas, s) {
            for &c in &e.stub {
                ruin_map.insert(c, full);
                plug_cells.insert(c);
            }
            clean_shapes.push(lip);
        }
    }
    (plug_cells, clean_shapes)
}

/// Jamb anchors for the outline's wall splice: for every dungeon room a
/// mouth or exit pierces, its opening on that room's wall (see [`Jamb`]).
/// Every jamb centre goes through [`clamp_opening`], so no gap crosses a
/// corner; the anchor room's jamb is exactly the mouth's (already-clamped)
/// centre, keeping the wall gap and the door bar in lockstep.
pub fn jambs(mouths: &[Mouth], topology: &Topology, areas: &Areas, s: f64) -> Vec<Jamb> {
    let mut out = Vec::new();
    for m in mouths {
        let mut rooms: Vec<usize> = m
            .members
            .iter()
            .flat_map(|&i| [topology.doors[i].a, topology.doors[i].b])
            .filter(|&r| areas.kind(r) == AreaKind::Dungeon)
            .collect();
        rooms.sort_unstable();
        rooms.dedup();
        // The door cells' centroid: the opening's true location, on whichever
        // wall each room presents to it.
        let dc = {
            let ps = m.members.iter().map(|&i| topology.doors[i].cell.center(s));
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            for p in ps {
                sx += p.0;
                sy += p.1;
                n += 1.0;
            }
            (sx / n, sy / n)
        };
        for r in rooms {
            if let Some(sh) = areas.shapes()[r] {
                let half = m.opening / 2.0;
                // The anchor room keeps the mouth's own centre so its wall gap
                // and the door bar stay in lockstep. Every *other* room picks
                // its pierced edge from the door cells' true location on its
                // own wall: `m.center` sits on the ANCHOR room's wall (often
                // laterally offset), so using it selects the wrong edge of a
                // corner-adjacent room and seals the passage behind a wall.
                let anchored_here = m.shape.is_none() || m.shape == Some(sh);
                let p = if anchored_here { m.center } else { dc };
                let out_r = wall_anchor(sh, p, None).map_or(m.out, |(_, o, _)| o);
                out.push(Jamb { shape: sh, center: clamp_opening(sh, p, half, out_r), half });
            }
        }
    }
    for e in &topology.exits {
        if areas.kind(e.area) == AreaKind::Dungeon && !e.stub.is_empty() {
            if let Some(sh) = areas.shapes()[e.area] {
                let half = HEX_APOTHEM * s;
                let p = e.stub[0].center(s);
                let out_e = wall_anchor(sh, p, None).map_or((0.0, 0.0), |(_, o, _)| o);
                let center = clamp_opening(sh, p, half, out_e);
                out.push(Jamb { shape: sh, center, half });
            }
        }
    }
    out
}

/// The straight throat for a dungeon room's exit passage: like [`plug`], but
/// through the room wall along the stub, so the exit mouth gets the same
/// crisp lip instead of bulging as raw locked hex cells. Returns the full
/// hall (projection: the whole stub straightens, fading organic with
/// distance) and the short **lip hall** at the mouth (clean decor: only the
/// doorframe skips hatching, not the whole passage).
fn exit_plug(e: &Exit, areas: &Areas, s: f64) -> Option<(RuinShape, RuinShape)> {
    if areas.kind(e.area) != AreaKind::Dungeon || e.stub.is_empty() {
        return None;
    }
    let sh = areas.shapes()[e.area]?;
    let first = e.stub[0].center(s);
    let last = e.stub[e.stub.len() - 1].center(s);
    let wall = sh.project(first);
    let u = (last.0 - wall.0, last.1 - wall.1);
    let len = u.0.hypot(u.1);
    let u = if len > 1e-6 {
        (u.0 / len, u.1 / len)
    } else {
        let v = (first.0 - wall.0, first.1 - wall.1);
        let len = v.0.hypot(v.1);
        if len < 1e-6 {
            return None;
        }
        (v.0 / len, v.1 / len)
    };
    let hw = HEX_APOTHEM * s;
    let (ax, ay) = (wall.0 - u.0 * 0.3 * s, wall.1 - u.1 * 0.3 * s);
    let full = RuinShape::StraightHall {
        ax,
        ay,
        bx: last.0 + u.0 * 1.0 * s,
        by: last.1 + u.1 * 1.0 * s,
        hw,
    };
    let lip = RuinShape::StraightHall {
        ax,
        ay,
        bx: wall.0 + u.0 * 1.0 * s,
        by: wall.1 + u.1 * 1.0 * s,
        hw,
    };
    Some((full, lip))
}

/// Anchor a mouth at `p` on the wall of `shape` it pierces: `(wall point,
/// outward normal, wall tangent)`. A rectangle's wall is chosen by the
/// passage direction `travel` when available (position ratios misclassify
/// corner-adjacent mouths), falling back to position; a circle's by position
/// alone. `None` for halls (never dungeon rooms) or a degenerate point.
fn wall_anchor(shape: RuinShape, p: Point, travel: Option<Point>) -> Option<(Point, Point, Point)> {
    match shape {
        RuinShape::Rect { cx, cy, hw, hh } => {
            let (dx, dy) = (p.0 - cx, p.1 - cy);
            // The pierced wall is the one `p` lies *outside* of. When `p` is
            // beyond exactly one edge, that edge is unambiguous — decisive on
            // position alone. Only a corner-adjacent point (outside both, or
            // inside both — e.g. a point already on a wall) needs the passage
            // direction / proportional ratio to disambiguate. Trusting travel
            // or the ratio outright misclassifies a door that sits squarely
            // off one wall but slightly past the far corner line.
            let (out_x, out_y) = (dx.abs() > hw, dy.abs() > hh);
            let through_flat = match (out_x, out_y) {
                (true, false) => false, // only past a side wall → left/right
                (false, true) => true,  // only past top/bottom → flat
                _ => match travel {
                    Some(u) => u.1.abs() >= u.0.abs(),
                    None => dy.abs() / hh >= dx.abs() / hw,
                },
            };
            Some(if through_flat {
                // Through the top/bottom wall.
                let sy = if dy >= 0.0 { 1.0 } else { -1.0 };
                ((cx + dx.clamp(-hw, hw), cy + hh * sy), (0.0, sy), (1.0, 0.0))
            } else {
                let sx = if dx >= 0.0 { 1.0 } else { -1.0 };
                ((cx + hw * sx, cy + dy.clamp(-hh, hh)), (sx, 0.0), (0.0, 1.0))
            })
        }
        RuinShape::Circle { cx, cy, .. } => {
            let n = (p.0 - cx, p.1 - cy);
            let len = n.0.hypot(n.1);
            (len > 1e-6).then(|| {
                let out = (n.0 / len, n.1 / len);
                (shape.project(p), out, (-out.1, out.0))
            })
        }
        _ => None,
    }
}

/// A door's unit passage direction (a-side to b-side neighbour centroid).
/// `None` if the door's two sides can't be located — never for a built map,
/// since every door touches both its areas.
fn passage_dir(d: &Door, areas: &Areas, s: f64) -> Option<Point> {
    let (mut a_acc, mut a_n) = ((0.0, 0.0), 0u32);
    let (mut b_acc, mut b_n) = ((0.0, 0.0), 0u32);
    for n in d.cell.neighbors() {
        let p = n.center(s);
        match areas.owner_of(n) {
            Some(o) if o == d.a => {
                a_acc = (a_acc.0 + p.0, a_acc.1 + p.1);
                a_n += 1;
            }
            Some(o) if o == d.b => {
                b_acc = (b_acc.0 + p.0, b_acc.1 + p.1);
                b_n += 1;
            }
            _ => {}
        }
    }
    if a_n == 0 || b_n == 0 {
        return None;
    }
    let a_c = (a_acc.0 / a_n as f64, a_acc.1 / a_n as f64);
    let b_c = (b_acc.0 / b_n as f64, b_acc.1 / b_n as f64);
    let u = (b_c.0 - a_c.0, b_c.1 - a_c.1);
    let len = u.0.hypot(u.1);
    if len < 1e-6 {
        return None;
    }
    Some((u.0 / len, u.1 / len))
}
