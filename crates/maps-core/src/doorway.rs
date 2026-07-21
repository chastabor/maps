//! Doorway mouths onto dungeon rooms.
//!
//! Doors whose cells are hex-adjacent carve one merged opening in the wall,
//! so doors are first clustered (union-find on cell adjacency) and each
//! dungeon-touching cluster becomes a [`Mouth`]: where the opening pierces
//! the room wall, which way it runs, and how wide it spans. Two consumers
//! share the mouths so they always agree:
//!
//! * the outline pipeline projects the mouth's door cells onto a straight
//!   **throat** ([`RuinShape::StraightHall`], see [`plug`]) perpendicular to
//!   the wall — the crisp doorway lip, fading organic down the corridor;
//! * the renderer draws the door glyph (bar + jamb caps) inside that lip.

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
    /// The mouth centre — see [`Anchor`].
    pub wall: Point,
    /// Unit direction out of the room, through the doorway.
    pub out: Point,
    /// Unit wall tangent — the direction the door bar runs.
    pub axis: Point,
    /// Member door-cell centres projected on `axis`, relative to `wall`,
    /// ascending. The mouth spans `[ts[0]-apothem, ts[last]+apothem]`.
    pub ts: Vec<f64>,
    /// Member cell centres' farthest reach along `out`, from `wall`.
    pub reach: f64,
    /// Wall-to-wall distance for `Midgap` (0 otherwise).
    pub gap: f64,
}

/// Cluster the map's doors into mouths. Only clusters that touch a dungeon
/// room produce one — other doors carve plain organic gaps and draw nothing.
pub fn mouths(topology: &Topology, areas: &Areas, s: f64) -> Vec<Mouth> {
    let find = crate::growth::find;
    let doors = &topology.doors;
    let mut root: Vec<usize> = (0..doors.len()).collect();
    // Two doors merge into one mouth only when they carve one opening: cells
    // hex-adjacent AND joining the same two areas. Adjacent cells serving
    // different pairs pierce different walls — merging them would hang one
    // long door bar across open floor between the two openings.
    let pair = |d: &Door| (d.a.min(d.b), d.a.max(d.b));
    for i in 0..doors.len() {
        for j in i + 1..doors.len() {
            if doors[i].cell.distance(doors[j].cell) <= 1 && pair(&doors[i]) == pair(&doors[j]) {
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
    let dungeon = |i: usize| areas.kind(i) == AreaKind::Dungeon;
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
    // Two rooms → anchored midway between the two pierced walls. Otherwise
    // (no shape / degenerate) → free: the mean passage direction, or the
    // nearest across-flats hex axis.
    let anchored = match rooms[..] {
        [r] => shapes[r]
            .and_then(|sh| wall_anchor(sh, c0, travel))
            .map(|(wall, out, axis)| (Anchor::Wall, wall, out, axis, 0.0)),
        [ra, rb] => shapes[ra].zip(shapes[rb]).and_then(|(sa, sb)| {
            let (wa, wb) = (sa.project(c0), sb.project(c0));
            let d = (wb.0 - wa.0, wb.1 - wa.1);
            let len = d.0.hypot(d.1);
            (len > 1e-6).then(|| {
                let out = (d.0 / len, d.1 / len);
                let wall = ((wa.0 + wb.0) / 2.0, (wa.1 + wb.1) / 2.0);
                (Anchor::Midgap, wall, out, (-out.1, out.0), len)
            })
        }),
        _ => None,
    };
    let (anchor, wall, out, axis, gap) = anchored.or_else(|| {
        if let Some(out) = travel {
            Some((Anchor::Free, c0, out, (-out.1, out.0), 0.0))
        } else {
            // Degenerate mean → nearest across-flats hex axis to the lead.
            let u = passage_dir(&doors[members[0]], areas, s)?;
            let t = (-u.1, u.0);
            let axis = DOOR_AXES.into_iter().max_by(|a, b| {
                (t.0 * a.0 + t.1 * a.1).abs().total_cmp(&(t.0 * b.0 + t.1 * b.1).abs())
            })?;
            Some((Anchor::Free, c0, (-axis.1, axis.0), axis, 0.0))
        }
    })?;

    let mut ts: Vec<f64> = centers
        .iter()
        .map(|p| (p.0 - wall.0) * axis.0 + (p.1 - wall.1) * axis.1)
        .collect();
    ts.sort_by(f64::total_cmp);
    let reach = centers
        .iter()
        .map(|p| (p.0 - wall.0) * out.0 + (p.1 - wall.1) * out.1)
        .fold(0.0, f64::max);
    Some(Mouth { members, anchor, wall, out, axis, ts, reach, gap })
}

/// Insert every doorway and dungeon-exit plug into `ruin_map`, so the
/// outline pipeline projects those cells onto their straight throats.
/// Returns `(plug_cells, lip_cells)`: all plugged cells (excluded from the
/// weathered ruin decor) and the subset that keeps the clean doorframe
/// treatment — door cells and each exit's mouth cell; an exit stub's
/// farther walls hatch like any organic passage.
pub fn apply_plugs(
    ruin_map: &mut HashMap<Hex, RuinShape>,
    mouths: &[Mouth],
    topology: &Topology,
    areas: &Areas,
    s: f64,
) -> (HashSet<Hex>, HashSet<Hex>) {
    let mut plug_cells = HashSet::new();
    let mut lip_cells = HashSet::new();
    for m in mouths {
        if let Some(hall) = plug(m, s) {
            for &i in &m.members {
                ruin_map.insert(topology.doors[i].cell, hall);
                plug_cells.insert(topology.doors[i].cell);
                lip_cells.insert(topology.doors[i].cell);
            }
        }
    }
    for e in &topology.exits {
        if let Some(hall) = exit_plug(e, areas, s) {
            for &c in &e.stub {
                ruin_map.insert(c, hall);
                plug_cells.insert(c);
            }
            lip_cells.insert(e.stub[0]);
        }
    }
    (plug_cells, lip_cells)
}

/// The straight throat a mouth's door cells project onto: a hall through the
/// wall gap, perpendicular to the wall, spanning the mouth. Its side walls
/// become the crisp doorway lip; down the corridor the projection fades
/// organic (the hall displacement fade in `outline::smooth_loops`). `None`
/// for free mouths — with no wall to frame, the opening stays organic.
fn plug(m: &Mouth, s: f64) -> Option<RuinShape> {
    if m.anchor == Anchor::Free {
        return None;
    }
    let ap = HEX_APOTHEM * s;
    let (t0, t1) = (m.ts[0] - ap, m.ts[m.ts.len() - 1] + ap);
    let mid = (t0 + t1) / 2.0;
    let c = (m.wall.0 + m.axis.0 * mid, m.wall.1 + m.axis.1 * mid);
    // Start just inside the wall and reach past the farthest member cell, so
    // every door-cell vertex projects sideways onto the jamb walls rather
    // than radially around an end cap.
    let back = m.gap / 2.0 + 0.3 * s;
    let fwd = (m.gap / 2.0).max(m.reach) + 1.2 * s;
    Some(RuinShape::StraightHall {
        ax: c.0 - m.out.0 * back,
        ay: c.1 - m.out.1 * back,
        bx: c.0 + m.out.0 * fwd,
        by: c.1 + m.out.1 * fwd,
        hw: (t1 - t0) / 2.0,
    })
}

/// The straight throat for a dungeon room's exit passage: like [`plug`], but
/// through the room wall along the stub, so the exit mouth gets the same
/// crisp lip instead of bulging as raw locked hex cells.
fn exit_plug(e: &Exit, areas: &Areas, s: f64) -> Option<RuinShape> {
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
    Some(RuinShape::StraightHall {
        ax: wall.0 - u.0 * 0.3 * s,
        ay: wall.1 - u.1 * 0.3 * s,
        bx: last.0 + u.0 * 1.0 * s,
        by: last.1 + u.1 * 1.0 * s,
        hw: HEX_APOTHEM * s,
    })
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
            let through_flat = match travel {
                Some(u) => u.1.abs() >= u.0.abs(),
                None => dy.abs() / hh >= dx.abs() / hw,
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
