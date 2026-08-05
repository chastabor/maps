//! Doorway mouths onto dungeon rooms.
//!
//! Doors whose cells are hex-adjacent carve one merged opening in the wall,
//! so doors are first clustered (union-find on cell adjacency) and each
//! dungeon-touching cluster becomes a [`Mouth`]: where the opening pierces
//! the room wall, which way it runs, and how wide it spans. Doorways are
//! **flush**: the opening is a gap cut into the room's exact wall (clamped so
//! it never crosses a rect corner — see `clamp_opening`) and nothing is
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
use crate::topology::{Connection, Exit, Topology};
use std::collections::{HashMap, HashSet};

/// √3/2 — the apothem of a unit-side hex (centre to edge midpoint). Half of
/// a single doorway's span, in hex-size units.
pub const HEX_APOTHEM: f64 = crate::grid::HEX_APOTHEM;

/// The three across-flats hex axes (edge-midpoint to opposite edge-midpoint),
/// as unit vectors at 0°, 60° and 120°. A mouth with no usable wall geometry
/// snaps to whichever its passage runs most nearly across.
const DOOR_AXES: [(f64, f64); 3] = [(1.0, 0.0), (0.5, HEX_APOTHEM), (-0.5, HEX_APOTHEM)];

/// A doorway's width, in hex sizes: one apothem per PAIR of shared edges, so 1–2 edges give one
/// door and 3–4 give one twice as wide.
///
/// A door's width should say nothing about which way its passage comes in. It used to be one hex
/// *width* per clustered door cell, capped at three, so the gap tracked how many cells happened to
/// cluster. Both sizes here are lattice constants, and so is the wall they sit on — see
/// [`porch_chord`]. `plans/tile-first-render.md` phase 3a.
const DOOR_OPENING: f64 = HEX_APOTHEM;

/// The two vertexes of the edge shared by neighbouring hexes `a` and `b`.
fn shared_edge(a: Hex, b: Hex, s: f64) -> Option<[Point; 2]> {
    let (ca, cb) = (a.corners(s), b.corners(s));
    let hit: Vec<Point> = ca
        .into_iter()
        .filter(|p| cb.iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < 1e-6))
        .collect();
    (hit.len() == 2).then(|| [hit[0], hit[1]])
}

/// The **porch**: the wall a doorway's leaf actually stands on, and the opening cut in it.
///
/// A room's border does not run along the tiles a passage attaches through — a rect's sits an
/// apothem *inside* its outer column — so a doorway cut straight into the border is a gap in the
/// wrong place and, once narrowed, a gap the floor does not match. The porch is the chord between
/// the outer vertexes of the door cells' shared edges with this room: the wall steps out to the
/// tile boundary, carries the door, and steps back.
///
/// The chord is a pure lattice constant — `s` across one shared edge, `√3·s` across two, `2s`
/// across three, with no orientation variation over 5925 measured attachments — which is what
/// makes one door size possible at all.
///
/// Returns `(chord, gap)` as `((A, B), (G0, G1))`, the gap centred on the chord with the leftover
/// split evenly into a jamb at each end.
fn porch_chord(
    cells: &[Hex],
    room: usize,
    areas: &Areas,
    s: f64,
) -> Option<((Point, Point), (Point, Point))> {
    let mut vs: Vec<Point> = Vec::new();
    let mut edges = 0usize;
    for &c in cells {
        for nb in c.neighbors() {
            if areas.owner_of(nb) != Some(room) {
                continue;
            }
            if let Some(e) = shared_edge(c, nb, s) {
                vs.extend(e);
                edges += 1;
            }
        }
    }
    if vs.is_empty() {
        return None;
    }
    // The chord spans the attachment: its two farthest-apart vertexes.
    let (mut a, mut b, mut span) = (vs[0], vs[0], 0.0f64);
    for p in &vs {
        for q in &vs {
            let d = (p.0 - q.0).hypot(p.1 - q.1);
            if d > span {
                span = d;
                a = *p;
                b = *q;
            }
        }
    }
    if span < 1e-6 {
        return None;
    }
    // One door per pair of edges, never wider than the chord can frame.
    let want = DOOR_OPENING * s * edges.div_ceil(2).max(1) as f64;
    let door = want.min(span - 0.1 * s);
    if door <= 0.0 {
        return None;
    }
    let f = (1.0 - door / span) / 2.0;
    let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    Some(((a, b), (at(f), at(1.0 - f))))
}

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
    /// Indices into `topology.connections`, ascending.
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
/// `blocked(p)` reports that wall point `p` is consumed by a fusion connector,
/// so no door may be cut there — the corridor already opens that stretch. Door
/// *cells* are chosen upstream in `topology::build`; only the opening's position
/// along the wall responds to this, so no randomness is involved.
pub fn mouths(
    topology: &Topology,
    areas: &Areas,
    s: f64,
    blocked: &dyn Fn(Point) -> bool,
) -> Vec<Mouth> {
    let find = crate::growth::find;
    let doors = &topology.connections;
    let dungeon = |i: usize| areas.kind(i) == AreaKind::Dungeon;
    let mut root: Vec<usize> = (0..doors.len()).collect();
    // Two doors merge into one mouth only when they carve one opening in one
    // room's wall: cells hex-adjacent AND sharing a dungeon room. (Adjacent
    // cells serving unrelated rooms pierce different walls — merging them
    // would hang one long door bar across open floor between the openings.)
    for i in 0..doors.len() {
        for j in i + 1..doors.len() {
            if doors[i].cell().distance(doors[j].cell()) <= 1
                && crate::topology::shared_dungeon_room(areas, &doors[i], &doors[j]).is_some()
            {
                let (a, b) = (find(&mut root, i), find(&mut root, j));
                root[a] = b;
            }
        }
    }
    // Also merge distance-2 pairs whose lone pillar the floor fills in — one
    // wide opening across the pillar (computed once on `Topology`).
    for &(i, j, _) in &topology.merged_doors {
        let (a, b) = (find(&mut root, i), find(&mut root, j));
        root[a] = b;
    }
    let mut clusters: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..doors.len() {
        let r = find(&mut root, i);
        clusters.entry(r).or_default().push(i);
    }
    clusters
        .into_values()
        .filter(|members| {
            members
                .iter()
                .any(|&i| dungeon(doors[i].a) || dungeon(doors[i].b))
        })
        .filter_map(|members| mouth(members, doors, areas, s, blocked))
        .collect()
}

fn mouth(
    members: Vec<usize>,
    doors: &[Connection],
    areas: &Areas,
    s: f64,
    blocked: &dyn Fn(Point) -> bool,
) -> Option<Mouth> {
    let dungeon = |i: usize| areas.kind(i) == AreaKind::Dungeon;
    // Every free-run cell of every member, not just each member's anchor: a connection is
    // CONNECTION_WIDTH cells wide, and the mouth must span and count what the passage actually
    // presents to the wall. Counting members alone pinned every opening at one hex — a two-cell
    // passage necked to a one-door gap at the wall it entered (reported on seed
    // 171030574712681231: a two-hex passageway with one-door openings both ends).
    let cells: Vec<crate::grid::Hex> = {
        let mut v: Vec<crate::grid::Hex> = members
            .iter()
            .flat_map(|&i| doors[i].along[..doors[i].apron_from].iter().copied())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let centers: Vec<Point> = cells.iter().map(|h| h.center(s)).collect();
    let c0 = centers
        .iter()
        .fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
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
                        Some(if da >= db {
                            flush(sa, (wa, oa, aa))
                        } else {
                            flush(sb, (wb, ob, ab))
                        })
                    }
                }
                (Some((sh, w)), None) | (None, Some((sh, w))) => Some(flush(sh, w)),
                _ => None,
            }
        }
        // 3+ dungeon rooms: a merged multi-door mouth (e.g. a narrow room with
        // two neighbours on one short wall). Anchor on the room every door
        // shares — the wall they all pierce — as one wide opening; the other
        // rooms sit beyond it. (No shared room → fall through to Free.)
        _ => {
            let common: Vec<usize> = rooms
                .iter()
                .copied()
                .filter(|&r| members.iter().all(|&i| doors[i].a == r || doors[i].b == r))
                .collect();
            match common[..] {
                [r] => shapes[r].and_then(|sh| wall_anchor(sh, c0, travel).map(|w| flush(sh, w))),
                _ => None,
            }
        }
    };
    let (anchor, wall, out, axis, anchor_shape) = anchored.or_else(|| {
        if let Some(out) = travel {
            Some((Anchor::Free, c0, out, (-out.1, out.0), None))
        } else {
            // Degenerate mean → nearest across-flats hex axis to the lead.
            let u = passage_dir(&doors[members[0]], areas, s)?;
            let t = (-u.1, u.0);
            let axis = DOOR_AXES.into_iter().max_by(|a, b| {
                (t.0 * a.0 + t.1 * a.1)
                    .abs()
                    .total_cmp(&(t.0 * b.0 + t.1 * b.1).abs())
            })?;
            Some((Anchor::Free, c0, (-axis.1, axis.0), axis, None))
        }
    })?;

    // One hex of opening per cell of passage, capped at a triple gate.
    let opening = cells.len().min(3) as f64 * 2.0 * ap;
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
    // A fusion connector replaces a stretch of this wall, so an opening cut
    // there would breach the corridor instead of the room. Slide it along the
    // wall to the nearest clear placement.
    // Bounded by the mouth's own cell span: the mouth exists to close THIS passage, so a
    // placement past the cells' extent is off-passage by definition. Unbounded (well,
    // opening-width-bounded) sliding walked a blocked Midgap mouth 62px along its axis and
    // stacked a triple gate against an unrelated room's wall (seed 171030574712681231, the
    // three doors on 5D) — a Midgap anchor has no shape, so nothing re-clamped the walk. A
    // span too blocked to clear within its own extent stays put and is subsumed by the
    // connector, which is this function's documented honest outcome.
    let max_slide = ((hi - lo) / 2.0 + ap - opening / 2.0).max(0.0);
    let center = slide_clear(
        anchor_shape,
        center,
        axis,
        out,
        opening / 2.0,
        max_slide,
        blocked,
    );
    Some(Mouth {
        members,
        anchor,
        center,
        out,
        axis,
        shape: anchor_shape,
        opening,
    })
}

/// Slide an opening along its wall until the whole span clears `blocked`,
/// nearest placement first in both directions, re-clamped to the same edge each
/// step (so a rectangle's gap still never crosses a corner and a circle's walks
/// around the arc). Returns `center` unchanged when it is already clear, and
/// also when nothing within a wall's length is — then the fusion opening simply
/// subsumes this door, which is the honest outcome rather than a forced move.
fn slide_clear(
    shape: Option<RuinShape>,
    center: Point,
    axis: Point,
    out: Point,
    half: f64,
    max_slide: f64,
    blocked: &dyn Fn(Point) -> bool,
) -> Point {
    // Sample the span's ends, quarters and centre: the blocked regions are
    // hex-scale and the span is at most three hexes, so this cannot step over one.
    let span_clear = |c: Point| {
        [-1.0, -0.5, 0.0, 0.5, 1.0]
            .iter()
            .all(|f| !blocked((c.0 + axis.0 * half * f, c.1 + axis.1 * half * f)))
    };
    if span_clear(center) {
        return center;
    }
    // Bounded to about one opening width. An unbounded walk pushed doors to the
    // far end of an edge (or saturated at a corner), which is what folded the
    // traced outline at the jamb; a door that cannot clear the connector within
    // one opening simply stays, and the connector yields to it instead.
    let step = (half / 2.0).max(1.0);
    (1..=4)
        .flat_map(|k| [-1.0, 1.0].map(|sgn| sgn * step * k as f64))
        .filter(|t| t.abs() <= max_slide)
        .map(|t| {
            let p = (center.0 + axis.0 * t, center.1 + axis.1 * t);
            match shape {
                Some(sh) => clamp_opening(sh, p, half, out),
                None => p,
            }
        })
        .find(|&c| span_clear(c))
        .unwrap_or(center)
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
                let y = if hh <= half {
                    cy
                } else {
                    p.1.clamp(cy - hh + half, cy + hh - half)
                };
                (cx + sx * hw, y)
            } else {
                // Top/bottom edge: y pinned to the wall, slide along x.
                let sy = if out.1 >= 0.0 { 1.0 } else { -1.0 };
                let x = if hw <= half {
                    cx
                } else {
                    p.0.clamp(cx - hw + half, cx + hw - half)
                };
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
    /// The porch, if this doorway stands on one: for each end of the opening, the chord vertex
    /// the wall steps out to and the gap edge the door starts at. `plus` is the end at wall
    /// parameter `tw + half`, `minus` the end at `tw - half`, so the splice can pick the side
    /// matching its walk direction without re-deriving the geometry.
    ///
    /// The wall run and the floor loop both grow these two points at the run's end, which is what
    /// keeps them agreeing: the floor necks to the door because the boundary itself steps out to
    /// the chord and back.
    pub plus: Option<(Point, Point)>,
    pub minus: Option<(Point, Point)>,
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
        if let Some((full, lip, projected)) = exit_plug(e, areas, s) {
            // Only the straight run projects onto the hall; a bent tail stays
            // organic (floor+narrow), so it renders as a wandering passage.
            for &c in &projected {
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
/// Every jamb centre goes through `clamp_opening`, so no gap crosses a
/// corner; the anchor room's jamb is exactly the mouth's (already-clamped)
/// centre, keeping the wall gap and the door bar in lockstep.
pub fn jambs(mouths: &[Mouth], topology: &Topology, areas: &Areas, s: f64) -> Vec<Jamb> {
    let mut out = Vec::new();
    for m in mouths {
        let mut rooms: Vec<usize> = m
            .members
            .iter()
            .flat_map(|&i| [topology.connections[i].a, topology.connections[i].b])
            .filter(|&r| areas.kind(r) == AreaKind::Dungeon)
            .collect();
        rooms.sort_unstable();
        rooms.dedup();
        // The door cells' centroid: the opening's true location, on whichever
        // wall each room presents to it.
        let dc = {
            let ps = m
                .members
                .iter()
                .map(|&i| topology.connections[i].cell().center(s));
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            for p in ps {
                sx += p.0;
                sy += p.1;
                n += 1.0;
            }
            (sx / n, sy / n)
        };
        let cells: Vec<Hex> = m
            .members
            .iter()
            .map(|&i| topology.connections[i].cell())
            .collect();
        for r in rooms {
            if let Some(sh) = areas.shapes()[r] {
                // The porch first: it decides where the opening is and how wide, so the border
                // gap it replaces is derived from it rather than the other way round.
                if let Some((chord, gap)) = porch_chord(&cells, r, areas, s) {
                    let (pa, pb) = (sh.project(chord.0), sh.project(chord.1));
                    let per = sh.perimeter().unwrap_or(0.0);
                    let (ta, tb) = (sh.wall_param(pa), sh.wall_param(pb));
                    let fwd = (tb - ta).rem_euclid(per);
                    // The shorter way round is the span the doorway opens; `from` is its start.
                    let (from, len) = if fwd <= per - fwd {
                        (ta, fwd)
                    } else {
                        (tb, per - fwd)
                    };
                    if len > 1e-6 {
                        // `minus` sits at `from`, `plus` at `from + len`; pair each with the
                        // chord vertex that projects to it and the nearer gap edge.
                        let near = |p: Point| {
                            if (p.0 - chord.0.0).hypot(p.1 - chord.0.1)
                                <= (p.0 - chord.1.0).hypot(p.1 - chord.1.1)
                            {
                                (chord.0, gap.0)
                            } else {
                                (chord.1, gap.1)
                            }
                        };
                        out.push(Jamb {
                            shape: sh,
                            center: sh.wall_point((from + len / 2.0).rem_euclid(per)),
                            half: len / 2.0,
                            minus: Some(near(sh.wall_point(from))),
                            plus: Some(near(sh.wall_point((from + len).rem_euclid(per)))),
                        });
                        continue;
                    }
                }
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
                out.push(Jamb {
                    shape: sh,
                    center: clamp_opening(sh, p, half, out_r),
                    half,
                    plus: None,
                    minus: None,
                });
            }
        }
    }
    for e in &topology.exits {
        if areas.kind(e.area) == AreaKind::Dungeon
            && !e.stub.is_empty()
            && let Some(sh) = areas.shapes()[e.area]
        {
            let half = HEX_APOTHEM * s;
            let p = e.stub[0].center(s);
            let out_e = wall_anchor(sh, p, None).map_or((0.0, 0.0), |(_, o, _)| o);
            let center = clamp_opening(sh, p, half, out_e);
            // Exits keep their throat plug (`exit_plug`) and so need no porch: the plug already
            // gives the passage a wall to meet.
            out.push(Jamb {
                shape: sh,
                center,
                half,
                plus: None,
                minus: None,
            });
        }
    }
    out
}

/// The straight throat for a dungeon room's exit passage: through the room wall
/// along the stub's **initial heading**, so the exit mouth gets a crisp lip
/// instead of bulging as raw locked hex cells. Only the contiguous run of stub
/// cells that stays within the straight hall band is projected; once the stub
/// bends away, the rest of it stays organic — a bent stub forced onto one
/// angled hall folds the passage shut into a pinched bulb. Returns the full
/// hall (projection over the straight run), the short **lip hall** at the mouth
/// (clean decor at the doorframe), and the stub cells to project.
fn exit_plug(e: &Exit, areas: &Areas, s: f64) -> Option<(RuinShape, RuinShape, Vec<Hex>)> {
    if areas.kind(e.area) != AreaKind::Dungeon || e.stub.is_empty() {
        return None;
    }
    let sh = areas.shapes()[e.area]?;
    let first = e.stub[0].center(s);
    let wall = sh.project(first);
    // Heading: the direction the stub leaves the wall (not wall→last, which a
    // bend would tilt off the straight run).
    let dir = (first.0 - wall.0, first.1 - wall.1);
    let len = dir.0.hypot(dir.1);
    let u = if len > 1e-6 {
        (dir.0 / len, dir.1 / len)
    } else {
        let last = e.stub[e.stub.len() - 1].center(s);
        let v = (last.0 - wall.0, last.1 - wall.1);
        let l = v.0.hypot(v.1);
        if l < 1e-6 {
            return None;
        }
        (v.0 / l, v.1 / l)
    };
    let hw = HEX_APOTHEM * s;
    // Project the contiguous run of stub cells within half a cell of the
    // straight ray; stop at the first cell that bends out of the band.
    let band = hw + 0.5 * s;
    let mut projected: Vec<Hex> = Vec::new();
    let mut max_t = 0.0_f64;
    for &c in &e.stub {
        let p = c.center(s);
        let (dx, dy) = (p.0 - wall.0, p.1 - wall.1);
        let t = dx * u.0 + dy * u.1;
        let d = (dx - t * u.0).hypot(dy - t * u.1);
        if t >= -0.6 * s && d <= band {
            projected.push(c);
            max_t = max_t.max(t);
        } else {
            break;
        }
    }
    projected.first()?;
    let (ax, ay) = (wall.0 - u.0 * 0.3 * s, wall.1 - u.1 * 0.3 * s);
    let full = RuinShape::StraightHall {
        ax,
        ay,
        bx: wall.0 + u.0 * (max_t + s),
        by: wall.1 + u.1 * (max_t + s),
        hw,
    };
    let lip = RuinShape::StraightHall {
        ax,
        ay,
        bx: wall.0 + u.0 * s,
        by: wall.1 + u.1 * s,
        hw,
    };
    Some((full, lip, projected))
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
                (
                    (cx + dx.clamp(-hw, hw), cy + hh * sy),
                    (0.0, sy),
                    (1.0, 0.0),
                )
            } else {
                let sx = if dx >= 0.0 { 1.0 } else { -1.0 };
                (
                    (cx + hw * sx, cy + dy.clamp(-hh, hh)),
                    (sx, 0.0),
                    (0.0, 1.0),
                )
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
fn passage_dir(d: &Connection, areas: &Areas, s: f64) -> Option<Point> {
    let (mut a_acc, mut a_n) = ((0.0, 0.0), 0u32);
    let (mut b_acc, mut b_n) = ((0.0, 0.0), 0u32);
    for n in d.cell().neighbors() {
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
