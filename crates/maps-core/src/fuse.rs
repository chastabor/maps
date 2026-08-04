//! Fusion connectors: the clean joins that replace the pinched, improvised
//! seam two independently-derived room shapes give where they grew into each
//! other.
//!
//! Three things happen here, in the order `generate_with` needs them:
//!
//! 1. **Classify** every fused pair ([`fuse_pairs`]) — which axis, if any, the
//!    two rooms line up on ([`FuseClass`]).
//! 2. **Construct** a connector per pair: [`axis_necks`] builds the wide straight
//!    corridor at the shapes' support clamp for every class and shape pair, and
//!    [`circle_rect_necks`] the narrow hex-aligned angle neck for a circle↔rect
//!    corner. Both emit a ladder of progressively narrower candidates, so a wall
//!    that crosses other geometry degrades instead of vanishing.
//! 3. **Splice** the accepted connector into both layers that draw a wall — the
//!    floor outline ([`splice_outline_necks`]) and the thick wall band
//!    ([`splice_necks`]) — through the one shared traversal, so the two can never
//!    disagree about where a corridor is.
//!
//! A corridor needs floor to stand on, which it gets during **growth**: [`corridor_floor`]
//! says where, `growth::claim_join_floor` decides what may be taken, and
//! [`commit_join_floor`] keeps only what a wall turns out to enclose.
//!
//! The design record, including the measurements behind the constants and the
//! approaches that were tried and rejected, is in `plans/fuse-case-taxonomy.md`.

use crate::AreaKind;
use crate::geom::{self, Point};
use crate::grid::{self, HexGrid};
use crate::growth::Areas;
use crate::outline::WallRun;
use crate::ruins;
use crate::topology::Topology;

/// Cells forming the "neck" of every narrowly-fused dungeon pair: two dungeon
/// rooms that grew cell-adjacent but touch across only one or two faces. The
/// neck is the touching cells of both rooms. The outline locks these on their
/// raw hex corners (rather than projecting each side onto its own pinching room
/// wall), so the join is a full-hex-width, hex-aligned neck — the two touching
/// hexes are already floor, so nothing new is filled. Rooms touching across ≥3
/// faces already read as one compound and contribute nothing.
///
/// A pair in `corridor_pairs` is left out: its seam is about to be spanned by a corridor
/// whose walls are drawn geometry, and the raw-hex lock fights them. The lock is for a
/// narrow seam that gets NO corridor, where the alternative is projecting the seam onto
/// one of the two rooms' pinching walls.
fn fused_necks(
    areas: &Areas,
    corridor_pairs: &std::collections::HashMap<(usize, usize), Vec<Point>>,
    s: f64,
) -> std::collections::HashSet<grid::Hex> {
    use std::collections::{HashMap, HashSet};
    let n = areas.count();
    let is_d = |i: usize| areas.kind(i) == AreaKind::Dungeon;
    // Touching cells per dungeon pair; the (cell, foreign-neighbour) adjacency
    // count is twice the seam's face width (each face is seen from both sides).
    let mut seam: HashMap<(usize, usize), (Vec<grid::Hex>, usize)> = HashMap::new();
    for a in 0..n {
        if !is_d(a) {
            continue;
        }
        for h in areas.floor_cells(a) {
            for nb in h.neighbors() {
                if let Some(b) = areas.owner_of(nb).filter(|&b| b != a && is_d(b)) {
                    let e = seam.entry((a.min(b), a.max(b))).or_default();
                    e.0.push(h);
                    e.1 += 1;
                }
            }
        }
    }
    let mut neck = HashSet::new();
    // A cell whose own room's border already CONTAINS it needs no lock, and taking one
    // actively hurts: the lock's whole premise is that the seam cells "lie outside that
    // room's geometry, and projecting undoes the fill". A tile-bounded circle (phase 1b)
    // inverts that — projecting a seam vertex onto the arc moves it OUTWARD onto the
    // border, undoing nothing — while the raw hex boundary it would be pinned to lies
    // inside the arc, so the wall band dips in to reach it: measured at up to 9.05px for a
    // 19-tile flower and 7.75px for a 7-tile one, which is the notch between two petals.
    //
    // A rect's border runs inside its tiles by design, so its seam cells fail this test and
    // keep the lock. With fitted circles the tiles poke out too, so nothing changes there.
    let contained = |c: grid::Hex| -> bool {
        areas
            .owner_of(c)
            .and_then(|i| areas.shape(i))
            .is_some_and(|sh| c.corners(s).iter().all(|v| sh.contains(*v)))
    };
    for (&pair, (cells, faces)) in &seam {
        if faces / 2 <= 2 && !corridor_pairs.contains_key(&pair) {
            // narrow touch, no corridor: give it a neck, minus cells their own room bounds
            neck.extend(cells.iter().copied().filter(|c| !contained(*c)));
        }
    }
    neck
}

/// Slop around a connector's footprint, in px: how far past its exact geometry
/// [`Neck::blocks`] still claims a vertex, and how far past the corridor span
/// [`in_claim_span`] still admits a cell. The two must agree — the claimed cells'
/// vertices have to fall inside the footprint that replaces them, or floor pokes
/// past a wall.
const FOOTPRINT_SLOP: f64 = 2.0;

/// Which construction produced a [`Neck`] — see plans/fuse-case-taxonomy.md.
/// This single discriminator drives every behaviour the two kinds differ in:
/// endpoint tagging in the splice, the footprint shape in [`Neck::blocks`],
/// outline-splice acceptance, floor claiming, and door policy (a narrow angle
/// neck nudges doors aside; a wide corridor yields to them).
#[derive(Clone, Copy, PartialEq, Eq)]
enum NeckKind {
    /// [`axis_necks`]: a wide straight corridor at the support clamp, its wall
    /// endpoints blended onto the room borders they meet. Any shape pair,
    /// classes B / C / D-diagonal / axis-aligned A.
    AxisCorridor,
    /// [`circle_rect_necks`]: the narrow hex-aligned angle neck for a
    /// circle↔rectangle corner fusion; its endpoints hug the hall centreline.
    AngleNeck,
    /// [`connection_necks`]: a room-to-room **link**. Spliced into neither layer — its wall is the
    /// link's own hex-edge boundary (see [`link_walls`]), which is tile-exact and needs no fitting.
    ///
    /// The neck still exists because `commit_join_floor` keeps join floor a neck's hall covers, and
    /// the link's floor must survive. Splicing its trapezoid as well produced the stray stubs at
    /// every passage mouth: a one-cell link's two "walls" are shorter than the wall band is thick,
    /// so they read as ticks at angles unrelated to the passage.
    Link,
}

/// A clean connector joining one fused pair of geometric areas, in place of the
/// pinched improvised join two independently-derived shapes give. The two
/// `line`s are the connector's outer walls, each stored as its own two
/// endpoints; `hall` is a `StraightHall` spanning between them so the renderer's
/// per-vertex inner offset draws the inner wall.
struct Neck {
    /// The two areas this connector fuses. Several candidates may be emitted for
    /// one pair (progressively shorter walls); the first accepted wins and the
    /// rest are skipped.
    pair: (usize, usize),
    shape_a: ruins::RuinShape,
    shape_b: ruins::RuinShape,
    lines: [(Point, Point); 2],
    hall: ruins::RuinShape,
    kind: NeckKind,
    /// The corridor floor growth claimed for this pair (see [`corridor_floor`]), as
    /// `(cell centre, circumradius)`.
    ///
    /// The splice replaces every outline/wall vertex inside the connector's
    /// footprint, and the hex lattice does not line up with the clamp: a claimed
    /// cell straddles the corridor wall, so some of its vertices fall outside the
    /// hall box. Left behind, they are floor poking past a wall. Counting each
    /// claimed cell's own disc as part of the footprint is what keeps the two in
    /// step — every vertex a claimed cell can contribute is a corner of its
    /// hexagon, hence within its circumradius (`cell`) — so the corridor ends up
    /// exactly as wide as its wall, whatever the lattice does.
    claimed: Vec<Point>,
    /// The hex cell size, so [`wall_stretch`](Neck::wall_stretch) can tell a wall
    /// end from a doorway jamb by the cell scale.
    cell: f64,
}

/// A connector's own coordinate frame: the origin at the hall's near end, its
/// length along the axis, and the unit axis and normal. Everything that reasons
/// about a connector works in these coordinates rather than in world x/y.
struct Frame {
    o: Point,
    len: f64,
    dir: Point,
    nrm: Point,
}

impl Frame {
    /// `p` in frame coordinates: `(along the axis from the origin, across it)`.
    /// The sign of the second component is which side of the centreline `p` is on.
    fn local(&self, p: Point) -> (f64, f64) {
        (
            (p.0 - self.o.0) * self.dir.0 + (p.1 - self.o.1) * self.dir.1,
            (p.0 - self.o.0) * self.nrm.0 + (p.1 - self.o.1) * self.nrm.1,
        )
    }
}

impl Neck {
    /// Whether this is the wide axis corridor. Its wall endpoints are tagged with
    /// the room shape they land on (blended), it participates in the outline
    /// splice, it claims corridor floor, and it yields to doors rather than
    /// nudging them.
    fn is_corridor(&self) -> bool {
        self.kind == NeckKind::AxisCorridor
    }

    /// Whether this is a room-to-room link, whose wall is drawn from its own tiles.
    fn is_link(&self) -> bool {
        self.kind == NeckKind::Link
    }

    /// The connector's axis as `(near end, far end)` — the centreline between its two
    /// walls. Both hall shapes reduce to it: a `StraightHall` *is* its centreline, and a
    /// `Trapezoid`'s is the segment joining its two caps' midpoints.
    fn axis(&self) -> Option<(Point, Point)> {
        match self.hall {
            ruins::RuinShape::StraightHall { ax, ay, bx, by, .. } => Some(((ax, ay), (bx, by))),
            ruins::RuinShape::Trapezoid { wall0, wall1 } => {
                let mid = |x: Point, y: Point| ((x.0 + y.0) / 2.0, (x.1 + y.1) / 2.0);
                Some((mid(wall0.0, wall1.0), mid(wall0.1, wall1.1)))
            }
            _ => None,
        }
    }

    /// Half the distance across the connector, measured perpendicular to its **walls**.
    ///
    /// Not in the axis frame: the two walls are parallel (each sits at a constant offset
    /// along the corridor normal) but unequal in length, so the axis joining the two caps'
    /// midpoints is *tilted* relative to them. Measuring the separation in that tilted
    /// frame would foreshorten it by the tilt and hand back a corridor narrower than the
    /// one the clamp chose.
    fn half_width(&self) -> Option<f64> {
        match self.hall {
            ruins::RuinShape::StraightHall { hw, .. } => Some(hw),
            ruins::RuinShape::Trapezoid { wall0, wall1 } => {
                let e = (wall0.1.0 - wall0.0.0, wall0.1.1 - wall0.0.1);
                let len = e.0.hypot(e.1);
                if len < 1e-9 {
                    return None;
                }
                // Distance from wall1's near end to wall0's infinite line.
                let v = (wall1.0.0 - wall0.0.0, wall1.0.1 - wall0.0.1);
                Some(((v.0 * e.1 - v.1 * e.0) / len).abs() / 2.0)
            }
            _ => None,
        }
    }

    /// The hall's frame, shared by everything that has to agree on where the
    /// connector sits.
    fn frame(&self) -> Option<Frame> {
        let ((ax, ay), (bx, by)) = self.axis()?;
        let d = (bx - ax, by - ay);
        let l = d.0.hypot(d.1).max(1e-9);
        Some(Frame {
            o: (ax, ay),
            len: l,
            dir: (d.0 / l, d.1 / l),
            nrm: (-d.1 / l, d.0 / l),
        })
    }

    /// Whether `p` falls inside the footprint this connector replaces — which
    /// outline/wall vertices the splice drops. The hall box, plus every hexagon
    /// whose floor the connector claimed (see [`Neck::claimed`]).
    fn blocks(&self, p: Point) -> bool {
        // A claimed cell's corners can reach past the hall box; its vertices are
        // the connector's to replace all the same.
        if self
            .claimed
            .iter()
            .any(|&c| (p.0 - c.0).hypot(p.1 - c.1) <= self.cell + 0.5)
        {
            return true;
        }
        let Some(hw) = self.half_width() else {
            return false;
        };
        let Some(f) = self.frame() else { return false };
        let (t, pp) = f.local(p);
        if pp.abs() > hw + FOOTPRINT_SLOP {
            return false;
        }
        if !self.is_corridor() {
            return t >= -FOOTPRINT_SLOP && t <= f.len + FOOTPRINT_SLOP;
        }
        let (near, far) = self.span_at(&f, pp);
        t >= near.min(far) - FOOTPRINT_SLOP && t <= near.max(far) + FOOTPRINT_SLOP
    }

    /// How far the connector reaches along its axis at cross-offset `pp`, as
    /// `(near end, far end)` in frame coordinates.
    ///
    /// The two walls are generally UNEQUAL — each runs until it meets a border, and a
    /// border curves — so the reach varies across the corridor. Interpolating between the
    /// walls' own endpoints gives the convex hull of the two wall segments, i.e. the
    /// trapezoid itself. A straight box between their midpoints would only be as long as
    /// their *average*: the longer wall's far end would stick out past it, outline
    /// vertices there would escape the splice, and the inserted wall would cross them.
    ///
    /// Shared by [`blocks`](Self::blocks) (which vertices the splice replaces) and
    /// [`claim_offset`] (which cells growth claims). Those two must agree — a claimed cell
    /// missing from the footprint keeps its vertices, and they are then floor past a wall.
    ///
    /// (The narrow angle neck's `lines` are not ordered near-end-first, so the
    /// interpolation is meaningless there; it keeps its hall box — see `blocks`.)
    fn span_at(&self, f: &Frame, pp: f64) -> (f64, f64) {
        let (n0, f0) = (f.local(self.lines[0].0), f.local(self.lines[0].1));
        let (n1, f1) = (f.local(self.lines[1].0), f.local(self.lines[1].1));
        let span = n1.1 - n0.1;
        let w = if span.abs() < 1e-9 {
            0.5
        } else {
            ((pp - n0.1) / span).clamp(0.0, 1.0)
        };
        (n0.0 + (n1.0 - n0.0) * w, f0.0 + (f1.0 - f0.0) * w)
    }

    /// Where a doorway cuts `line`, as the jamb edge nearest `from`: the door cell's
    /// centre projected onto the wall, pulled back by the cell's own half-width.
    ///
    /// A band run that ends at a doorway has no neighbour to clip against, and its own
    /// last vertex sits a whole cell short of the jamb — the jamb vertex belongs to the
    /// (non-dungeon) door cell, which is exactly why the run ended. So the doorway has
    /// to be asked directly. This is the same `centre ± half` rule
    /// `outline::splice_dungeon_runs` snaps room walls to; it just reads the door cell
    /// rather than a room's wall anchor, because the wall being cut here is a
    /// connector's, not a room's.
    ///
    /// Only doors that actually cross this wall count: a door cell whose floor reaches
    /// the line has its centre within one cell of it (the cell reaches its
    /// circumradius), and its projection must fall inside the wall's span.
    fn jamb_edge(&self, line: (Point, Point), doors: &[Point], from: Point) -> Option<Point> {
        let d = (line.1.0 - line.0.0, line.1.1 - line.0.1);
        let len = d.0.hypot(d.1);
        if len < 1e-9 {
            return None;
        }
        let (u, nrm) = ((d.0 / len, d.1 / len), (-d.1 / len, d.0 / len));
        let par = |p: Point| (p.0 - line.0.0) * u.0 + (p.1 - line.0.1) * u.1;
        let perp = |p: Point| ((p.0 - line.0.0) * nrm.0 + (p.1 - line.0.1) * nrm.1).abs();
        let half = grid::HEX_APOTHEM * self.cell;
        let t_from = par(from);
        doors
            .iter()
            .filter(|&&c| perp(c) <= self.cell * 1.05)
            .map(|&c| par(c))
            .filter(|&t| t > 0.0 && t < len)
            // The edge on the side the run approaches from, and only ahead of it.
            .map(|t| if t >= t_from { t - half } else { t + half })
            .filter(|&e| e > 0.0 && e < len && (e - t_from).abs() < 2.0 * self.cell)
            .min_by(|a, b| (a - t_from).abs().total_cmp(&(b - t_from).abs()))
            .map(|e| (line.0.0 + u.0 * e, line.0.1 + u.1 * e))
    }

    /// The stretch of side-`line` wall that replaces one dropped run of outline
    /// vertices, as its two endpoints in traversal order.
    ///
    /// The run is clipped to what it actually covered, rather than always emitting
    /// the wall whole. A doorway crossing the wall splits the run in two, and
    /// emitting the whole wall for each half sends the border out to the far end and
    /// straight back to pick up the jamb — a long spike across the corridor mouth,
    /// which is what this fixes. Where a run does reach a wall end (the ordinary
    /// case, and every run at a room border) the clip lands exactly on it, so an
    /// uninterrupted wall is emitted exactly as before.
    ///
    /// `prev` and `next` are the kept vertices either side of the run; the returned
    /// `bool`s say whether each endpoint is the wall's own end, which is where the
    /// wall meets a room border and so may be tagged with that room's shape.
    fn wall_stretch(
        &self,
        line: (Point, Point),
        prev: Point,
        next: Point,
    ) -> ((Point, bool), (Point, bool)) {
        let d = (line.1.0 - line.0.0, line.1.1 - line.0.1);
        let len = d.0.hypot(d.1).max(1e-9);
        // A neighbour within HALF A CELL of a wall end is that end: the clip must
        // only ever SPLIT a wall, never shorten it away from a room, and a smoothed
        // corner vertex by the corridor mouth sits a jitter's width inside. Half a
        // cell covers jitter (a few units) without reaching a jamb, which is a whole
        // doorway further in.
        let tol = (self.cell / 2.0 / len).min(0.5);
        let param = |p: Point| {
            let (t, _) = geom::project_on_segment(p, line.0, line.1);
            if t < tol {
                0.0
            } else if t > 1.0 - tol {
                1.0
            } else {
                t
            }
        };
        let (s0, s1) = (param(prev), param(next));
        let at = |t: f64| geom::lerp(line.0, line.1, t);
        (
            (at(s0), s0 == 0.0 || s0 == 1.0),
            (at(s1), s1 == 0.0 || s1 == 1.0),
        )
    }
}

/// Support interval of a shape projected onto unit normal `n`: the shape spans
/// `[c·n − e, c·n + e]` along `n`. `None` for halls, which are connectors, not
/// rooms.
fn support(sh: &ruins::RuinShape, n: (f64, f64)) -> Option<(f64, f64)> {
    match *sh {
        ruins::RuinShape::Circle { cx, cy, r } => {
            let c = cx * n.0 + cy * n.1;
            Some((c - r, c + r))
        }
        ruins::RuinShape::Rect { cx, cy, hw, hh } => {
            let c = cx * n.0 + cy * n.1;
            let e = (hw * n.0).abs() + (hh * n.1).abs();
            Some((c - e, c + e))
        }
        _ => None,
    }
}

/// Where the line `p·n = u` crosses this shape's border, as a coordinate along
/// `d` (the corridor axis), on the side facing `sgn`. `n` and `d` are orthonormal
/// and axis-aligned. `None` when the line misses the shape — which the support
/// clamp is precisely what prevents.
fn border_along(
    sh: &ruins::RuinShape,
    n: (f64, f64),
    u: f64,
    d: (f64, f64),
    sgn: f64,
) -> Option<f64> {
    match *sh {
        ruins::RuinShape::Circle { cx, cy, r } => {
            let (cn, cd) = (cx * n.0 + cy * n.1, cx * d.0 + cy * d.1);
            let h2 = r * r - (cn - u) * (cn - u);
            (h2 >= 0.0).then(|| cd + sgn * h2.sqrt())
        }
        ruins::RuinShape::Rect { cx, cy, hw, hh } => {
            let (cn, cd) = (cx * n.0 + cy * n.1, cx * d.0 + cy * d.1);
            let e_n = (hw * n.0).abs() + (hh * n.1).abs();
            let e_d = (hw * d.0).abs() + (hh * d.1).abs();
            ((cn - u).abs() <= e_n + 1e-9).then_some(cd + sgn * e_d)
        }
        _ => None,
    }
}

/// A hex tile's extent along unit direction `n`, centre to extreme.
fn hex_support(n: Point, s: f64) -> f64 {
    (0..6)
        .map(|k| {
            let c = grid::hex_corner((0.0, 0.0), k, s);
            c.0 * n.0 + c.1 * n.1
        })
        .fold(f64::MIN, f64::max)
}

/// The extent along `n` of the tiles where the two areas actually touch — the ground the
/// corridor has to cover, measured in tiles rather than in fitted geometry.
///
/// The support clamp (`support(sa) ∩ support(sb)`) is derived from the fitted *shapes*, and a
/// fitted shape stops most of a cell short of its own outermost tiles. So the clamp can miss
/// a contact row entirely: the corridor then covers one row exactly and leaves the next one
/// outside its walls, where the seam keeps its organic boundary and the band chords across
/// the mouth. Each contact tile contributes its **whole** extent, so a wall placed at the
/// result bounds the tile instead of bisecting it.
fn contact_span(areas: &Areas, a: usize, b: usize, n: Point, s: f64) -> Option<(f64, f64)> {
    let e = hex_support(n, s);
    // Each side's own contact tiles, then the INTERSECTION. Both rooms must have floor across
    // the whole range or the corridor would span an offset where only one of them is there,
    // and that wall runs out of the pair entirely — measured, it costs 39 connectors and puts
    // walls 9px inside neighbouring rooms.
    let side = |x: usize, y: usize| -> Option<(f64, f64)> {
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for c in areas.floor_cells(x) {
            if c.neighbors()
                .iter()
                .any(|nb| areas.owner_of(*nb) == Some(y))
            {
                let p = c.center(s);
                let un = p.0 * n.0 + p.1 * n.1;
                lo = lo.min(un - e);
                hi = hi.max(un + e);
            }
        }
        (hi > lo).then_some((lo, hi))
    };
    let (a_lo, a_hi) = side(a, b)?;
    let (b_lo, b_hi) = side(b, a)?;
    let (lo, hi) = (a_lo.max(b_lo), a_hi.min(b_hi));
    (hi > lo).then_some((lo, hi))
}

/// The contact tiles' extent in a corridor's own `(n, d)` frame, each tile counted whole:
/// `((across_lo, across_hi), (along_lo, along_hi))`.
///
/// The across-range is [`contact_span`]'s — the intersection of the two sides, so the corridor
/// only spans offsets where both rooms have floor. The along-range is the **union** of both
/// sides' contact tiles, because that is the ground the corridor has to bridge: from the far
/// edge of one room's contact tiles to the far edge of the other's.
///
/// Purely tile data. No fitted border is consulted, so a corridor built from this exists for
/// every pair that touches, which is the property [`contact_necks`] needs.
fn contact_box(
    areas: &Areas,
    a: usize,
    b: usize,
    n: Point,
    d: Point,
    s: f64,
) -> Option<((f64, f64), (f64, f64))> {
    let across = contact_span(areas, a, b, n, s)?;
    let ed = hex_support(d, s);
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for (x, y) in [(a, b), (b, a)] {
        for c in areas.floor_cells(x) {
            if c.neighbors()
                .iter()
                .any(|nb| areas.owner_of(*nb) == Some(y))
            {
                let p = c.center(s);
                let ud = p.0 * d.0 + p.1 * d.1;
                lo = lo.min(ud - ed);
                hi = hi.max(ud + ed);
            }
        }
    }
    (hi > lo).then_some((across, (lo, hi)))
}

/// A corridor per [`Connection`](crate::topology::Connection), with its walls taken from the run of
/// cells the connection occupies.
///
/// The one construction for room-to-room links, and the reason the object exists. Everything it
/// needs is on the connection: `topology` decided the edge and reserved its cells, so the walls are
/// simply the two sides of that run — its extent across the corridor, each cell counted whole.
/// Nothing to negotiate with another pass and nothing to fall back to, because it cannot fail for a
/// connection whose cells were claimed.
///
/// It cannot come from `fuse_pairs`: after that (correctly) keys on `room_cells`, a pair joined by a
/// corridor is not a fused pair, and should not be — fusing them merges the two rooms into one
/// compound and crops the wall between them away.
///
/// The axis is the one best aligned with the direction between the two rooms, not the widest
/// contact: a corridor runs **from** one room **to** the other. Its length reaches a cell past the
/// run at each end so both walls cross both borders; [`trim_to_gap`] cuts them back to the gap and
/// [`clip::split_outside`](crate::clip::split_outside) removes whatever still lies inside a room.
fn connection_necks(
    areas: &Areas,
    connections: &[crate::topology::Connection],
    s: f64,
) -> Vec<Neck> {
    const AP: f64 = grid::HEX_APOTHEM;
    const AXES: [(Point, Point); 4] = [
        ((1.0, 0.0), (0.0, 1.0)),
        ((0.0, 1.0), (1.0, 0.0)),
        ((AP, -0.5), (0.5, AP)),
        ((AP, 0.5), (-0.5, AP)),
    ];
    let centroid = |i: usize| -> Option<Point> {
        let v: Vec<Point> = areas.room_cells(i).map(|c| c.center(s)).collect();
        (!v.is_empty()).then(|| {
            let n = v.len() as f64;
            (
                v.iter().map(|p| p.0).sum::<f64>() / n,
                v.iter().map(|p| p.1).sum::<f64>() / n,
            )
        })
    };
    let mut out = Vec::new();
    for conn in connections {
        let (a, b) = (conn.a, conn.b);
        let (Some(sa), Some(sb)) = (areas.shape(a), areas.shape(b)) else {
            continue;
        };
        // Only the cells still claimed: `ruins::erode` and `shrink_corridors` may have marked some
        // back to rock, and a wall must bound the floor that is actually there.
        let cells: Vec<Point> = conn
            .along
            .iter()
            .filter(|c| areas.is_join(**c))
            .map(|c| c.center(s))
            .collect();
        if cells.is_empty() {
            continue;
        }
        let (Some(ca), Some(cb)) = (centroid(a), centroid(b)) else {
            continue;
        };
        let want = (cb.0 - ca.0, cb.1 - ca.1);
        let Some(&(d, n)) = AXES.iter().max_by(|x, y| {
            let dot = |ax: Point| (ax.0 * want.0 + ax.1 * want.1).abs();
            dot(x.0).total_cmp(&dot(y.0))
        }) else {
            continue;
        };
        let (en, ed) = (hex_support(n, s), hex_support(d, s));
        let (mut n_lo, mut n_hi) = (f64::MAX, f64::MIN);
        let (mut d_lo, mut d_hi) = (f64::MAX, f64::MIN);
        for p in &cells {
            let (un, ud) = (p.0 * n.0 + p.1 * n.1, p.0 * d.0 + p.1 * d.1);
            n_lo = n_lo.min(un - en);
            n_hi = n_hi.max(un + en);
            d_lo = d_lo.min(ud - ed);
            d_hi = d_hi.max(ud + ed);
        }
        if n_hi <= n_lo {
            continue;
        }
        let (d_lo, d_hi) = (d_lo - s, d_hi + s);
        let at = |u: f64, t: f64| (n.0 * u + d.0 * t, n.1 * u + d.1 * t);
        let wall = |u: f64| (at(u, d_lo), at(u, d_hi));
        let (w0, w1) = (wall(n_lo), wall(n_hi));
        out.push(Neck {
            pair: (a, b),
            shape_a: sa,
            shape_b: sb,
            lines: [w0, w1],
            hall: ruins::RuinShape::Trapezoid {
                wall0: w0,
                wall1: w1,
            },
            kind: NeckKind::Link,
            claimed: cells,
            cell: s,
        });
    }
    out
}

/// One cell's hex-edge wall runs: an edge is wall unless `is_mouth` says it opens.
///
/// Edge `k` spans corner `k` to corner `k+1` — see `outline`'s `D`. Each vertex carries the
/// cell's own hexagon, so the renderer's inward offset follows the lattice. Shared by
/// [`link_walls`] and [`junction_walls`], which differ only in their mouth rule — the
/// edge-to-corner correspondence is load-bearing and lives here once.
fn hex_edge_walls(
    c: grid::Hex,
    s: f64,
    is_mouth: impl Fn(grid::Hex) -> bool,
    out: &mut Vec<WallRun>,
) {
    let ctr = c.center(s);
    let hex = ruins::RuinShape::HexCell {
        cx: ctr.0,
        cy: ctr.1,
        s,
    };
    let corners = c.corners(s);
    for (k, nb) in c.neighbors().into_iter().enumerate() {
        if is_mouth(nb) {
            continue;
        }
        out.push(vec![
            (crate::outline::quantize_pt(corners[k]), hex),
            (crate::outline::quantize_pt(corners[(k + 1) % 6]), hex),
        ]);
    }
}

/// Stitch hex-edge segments that share endpoints into maximal polylines.
///
/// [`hex_edge_walls`] emits one two-point run per edge, and the renderer closes **each run as
/// its own capsule** — outer line forward, inner offset back. Three consecutive edges of one
/// cell therefore drew as three overlapping 12x7.2px wedges whose corners crossed instead of
/// mitring: the "bowtie" tangle visible at every link mouth. Joined into one polyline, the
/// renderer mitres the interior corners and draws one clean band.
///
/// Where two segments from **different cells** meet, the shared corner is kept twice — once
/// with each cell's hexagon — which is exactly the coincident-vertex convention the renderer's
/// mitre pass already handles for neck corners (`render`'s `wall_band_layer`). Same-cell
/// segments merge into a single vertex.
///
/// Endpoints are compared exactly: both sides come out of `quantize_pt`, and the corner of a
/// lattice cell is computed identically by each cell that shares it. Walking order is by
/// segment index, so the output is seed-stable.
fn stitch_hex_edges(segs: Vec<WallRun>) -> Vec<WallRun> {
    let key = |p: Point| (p.0.to_bits(), p.1.to_bits());
    let mut used = vec![false; segs.len()];
    // Endpoint -> (segment, which end). Vec-scanned; a map would be faster but the pool is a
    // handful of edges per map.
    let mut out = Vec::new();
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        // The chain under construction, as (point, shape) with the start segment's direction.
        let mut chain: Vec<(Point, ruins::RuinShape)> = segs[start].clone();
        // Extend at the tail, then at the head, until neither end continues.
        loop {
            let mut grew = false;
            for i in 0..segs.len() {
                if used[i] {
                    continue;
                }
                let (a, b) = (segs[i][0], segs[i][1]);
                let head = chain[0];
                let tail = chain[chain.len() - 1];
                let (attach_tail, next) = if key(a.0) == key(tail.0) {
                    (true, b)
                } else if key(b.0) == key(tail.0) {
                    (true, a)
                } else if key(a.0) == key(head.0) {
                    (false, b)
                } else if key(b.0) == key(head.0) {
                    (false, a)
                } else {
                    continue;
                };
                used[i] = true;
                grew = true;
                let joint = if attach_tail { tail } else { head };
                // Different owning cells: keep the corner twice, once per shape, so the
                // renderer mitres the inner offset there instead of drooping.
                let seam = next.1 != joint.1;
                if attach_tail {
                    if seam {
                        chain.push((joint.0, next.1));
                    }
                    chain.push(next);
                } else {
                    if seam {
                        chain.insert(0, (joint.0, next.1));
                    }
                    chain.insert(0, next);
                }
            }
            if !grew {
                break;
            }
        }
        out.push(chain);
    }
    out
}

/// The wall of every room-to-room **link**: its own floor's boundary, as hex edges.
///
/// Tile-exact by construction, and the reason a link needs no fitted geometry at all. An edge of a
/// link cell is wall unless it faces the link's other floor or one of the two rooms it joins —
/// those are its mouths, where the passage opens.
///
/// Emitted as `dungeon_walls` runs, so a link is drawn with the dungeon's own double border rather
/// than an organic single stroke: a corridor between two dungeon rooms is part of the dungeon.
fn link_walls(areas: &Areas, connections: &[crate::topology::Connection], s: f64) -> Vec<WallRun> {
    let mut out = Vec::new();
    for conn in connections {
        // A `BTreeSet`, not a `HashSet`: the cells are iterated to emit wall runs, and hash
        // order varies per process — harmless while a link was one cell, byte-nondeterministic
        // the moment the apron made it two.
        let own: std::collections::BTreeSet<grid::Hex> = conn
            .along
            .iter()
            .copied()
            .filter(|c| areas.is_join(*c))
            .collect();
        if own.is_empty() {
            continue;
        }
        for &c in &own {
            // Its own floor, or a room it joins: a mouth, not a wall.
            let is_mouth = |nb: grid::Hex| {
                own.contains(&nb)
                    || matches!(areas.owner_of(nb), Some(o) if o == conn.a || o == conn.b)
            };
            hex_edge_walls(c, s, is_mouth, &mut out);
        }
    }
    out
}

/// Cells where three or more areas meet. A junction is its **own** section, not part of any
/// pair's.
///
/// Growth claims such a cell for one pair, but no single pair's connector window reaches it —
/// that is precisely why it survived as an orphan (all 6 residual cases over 1800 dense maps had
/// this shape: the cell touching its owner plus two other areas, with a link neck and a corridor
/// neck existing for those pairs and neither covering the cell). Treated as a section of its own
/// it is spoken for by definition and walls itself from its own hex edges.
///
/// Derived from the current `areas` at the point it is needed, per `outline`'s join-kind rule —
/// junction-ness is time-varying (erosion or a later claim can change the neighbour count), so
/// it must never be snapshotted early. `pub(crate)` so the outline's `JoinKind` map and the wall
/// builder classify from the SAME source.
pub(crate) fn junction_cells(areas: &Areas) -> Vec<grid::Hex> {
    let mut out: Vec<grid::Hex> = areas
        .join()
        .iter()
        .copied()
        .filter(|c| {
            let owners: std::collections::BTreeSet<usize> = c
                .neighbors()
                .iter()
                .filter_map(|n| areas.owner_of(*n))
                .chain(areas.owner_of(*c))
                .collect();
            owners.len() >= 3
        })
        .collect();
    // `join` is a `HashSet`; sort so the wall runs land in a seed-stable order.
    out.sort_unstable();
    out
}

/// A junction's wall: its own hex edges, walling only the ones facing **rock**.
///
/// An edge facing floor is a mouth — the junction opens into that area, and a wall drawn across
/// open floor is the defect `clip::split_outside` exists to remove. Same construction as
/// [`link_walls`], which is what makes the junction enclosable without any pair agreeing to it.
fn junction_walls(areas: &Areas, s: f64) -> Vec<WallRun> {
    let mut out = Vec::new();
    for c in junction_cells(areas) {
        hex_edge_walls(c, s, |nb| areas.owner_of(nb).is_some(), &mut out);
    }
    out
}

/// A corridor for every fused pair that no other construction could build one for.
///
/// The other constructions end a wall where it meets a room's fitted *border*, and at the full
/// tile-derived width that border may present nothing to end on — neither a crossing nor a tile
/// the wall line passes through. The pair then gets **no connector at all**, and if its seam is
/// not covered by the two rooms' own borders it is left unwalled (measured: 2 pairs of 687 over
/// 200 seeds, both rect↔rect, both with the seam midpoint half an apothem outside both rects).
///
/// This builds the same [`Neck`] from the tiles that touch and nothing else, so it cannot fail
/// for a pair that touches at all. A corridor then exists between every fused pair, which is
/// worth more than covering those two seams: it is the object that says *these two borders need
/// clipping*, so the render has one thing to ask rather than having to rediscover the
/// relationship from the shapes.
fn contact_necks(
    areas: &Areas,
    pairs: &[(usize, usize, FuseClass)],
    covered: &[(usize, usize)],
    s: f64,
) -> Vec<Neck> {
    const AP: f64 = grid::HEX_APOTHEM;
    // Every axis the other constructions use, so the widest contact wins as it does there.
    const AXES: [(Point, Point); 4] = [
        ((1.0, 0.0), (0.0, 1.0)),
        ((0.0, 1.0), (1.0, 0.0)),
        ((AP, -0.5), (0.5, AP)),
        ((AP, 0.5), (-0.5, AP)),
    ];
    let mut out = Vec::new();
    for &(a, b, _) in pairs {
        if covered.contains(&(a, b)) {
            continue;
        }
        let (Some(sa), Some(sb)) = (areas.shape(a), areas.shape(b)) else {
            continue;
        };
        let Some((&(d, n), across, along)) = AXES
            .iter()
            .filter_map(|ax @ &(d, n)| {
                contact_box(areas, a, b, n, d, s).map(|(ac, al)| (ax, ac, al))
            })
            .max_by(|x, y| (x.1.1 - x.1.0).total_cmp(&(y.1.1 - y.1.0)))
        else {
            continue;
        };
        let at = |u: f64, t: f64| (n.0 * u + d.0 * t, n.1 * u + d.1 * t);
        let wall = |u: f64| (at(u, along.0), at(u, along.1));
        let hall = ruins::RuinShape::Trapezoid {
            wall0: wall(across.0),
            wall1: wall(across.1),
        };
        out.push(Neck {
            pair: (a, b),
            shape_a: sa,
            shape_b: sb,
            lines: [wall(across.0), wall(across.1)],
            hall,
            kind: NeckKind::AxisCorridor,
            claimed: Vec::new(),
            cell: s,
        });
    }
    out
}

/// Where the wall line `p·n = u` leaves one hex TILE, as a coordinate along `d`, on the
/// `sgn` side — or `None` if the line misses the tile.
///
/// A pointy-top hex is the intersection of three slabs: edge normals at 0° and ±60°, each an
/// apothem from the centre. Clipping the line against all three gives the interval it spans
/// inside the tile, and the answer is that interval's `sgn` end.
fn tile_exit(centre: Point, n: Point, u: f64, d: Point, sgn: f64, s: f64) -> Option<f64> {
    const AP: f64 = grid::HEX_APOTHEM;
    let a = AP * s;
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    for m in [(1.0, 0.0), (0.5, AP), (-0.5, AP)] {
        // A point on the line is `u·n + t·d`, so its offset on normal `m` is `aj + t·bj`.
        let aj = u * (n.0 * m.0 + n.1 * m.1) - (centre.0 * m.0 + centre.1 * m.1);
        let bj = d.0 * m.0 + d.1 * m.1;
        if bj.abs() < 1e-9 {
            // Parallel to this slab: either wholly inside it or the line misses the tile.
            if aj.abs() > a {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((-a - aj) / bj, (a - aj) / bj);
        lo = lo.max(t0.min(t1));
        hi = hi.min(t0.max(t1));
    }
    (hi >= lo).then_some(if sgn >= 0.0 { hi } else { lo })
}

/// [`border_along`]'s tile-based counterpart: how far area `i`'s **floor** reaches along `d`
/// on the wall line `p·n = u`, for when the fitted shape does not reach that line at all.
///
/// A fitted circle stops at `max_cell_centre_distance + 0.4·s`, most of a cell short of the
/// area's own outermost tiles, so a wall placed to bound those tiles has no arc to end on.
/// The tile edge is the honest endpoint there — it is ground the area really holds, and it is
/// a hex edge the room and the corridor share, which is what lets the band join the corridor's
/// wall to the room's arc instead of chording across the mouth.
fn tile_border_along(
    areas: &Areas,
    i: usize,
    n: Point,
    u: f64,
    d: Point,
    sgn: f64,
    s: f64,
) -> Option<f64> {
    areas
        .floor_cells(i)
        .filter_map(|c| tile_exit(c.center(s), n, u, d, sgn, s))
        .reduce(|x, y| if sgn >= 0.0 { x.max(y) } else { x.min(y) })
}

/// Rows and vertical columns occupied by an area's **room** cells. `interior_only`
/// keeps just the cells no wall cuts (all six neighbours in the same area).
///
/// Corridor floor is skipped, so classification sees the room as it grew. It is the
/// room's floor but not part of its ROOM: a corridor stretches an area's rows and
/// columns towards its partner, and counting those would let a pair's own corridor
/// reclassify the pair that produced it (`fuse::plan` runs after growth claimed it).
///
/// Row key is `r` (constant `r` is constant `y`: a row of adjacent cells). The
/// column key is `2q + r`, NOT `q` — with `x = √3·s·(q + r/2)` a constant `q`
/// shifts x by `√3·s/2` per row, i.e. it is a DIAGONAL; constant `x` means
/// constant `2q + r`.
fn rows_cols(
    areas: &Areas,
    i: usize,
    interior_only: bool,
) -> (
    std::collections::HashSet<i32>,
    std::collections::HashSet<i32>,
) {
    let (mut rows, mut cols) = (
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    );
    for h in areas.room_cells(i) {
        if interior_only
            && !h
                .neighbors()
                .iter()
                .all(|nb| areas.owner_of(*nb) == Some(i))
        {
            continue;
        }
        rows.insert(h.r);
        cols.insert(2 * h.q + h.r);
    }
    (rows, cols)
}

/// Which connector a fused pair needs. The split rule: class **D** is tested
/// with *all* cells (a border-cut outer tile still overlaps), everything else
/// with interior cells only. See plans/fuse-case-taxonomy.md.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FuseClass {
    /// Neither axis lines up — a corner fusion, needs an angle neck.
    Angle,
    /// A shared row: the two line up horizontally, so a level corridor joins them.
    Horiz,
    /// A shared column: the vertical analogue.
    Vert,
    /// Both axes overlap: already open, so only the barrier wants cropping.
    Both,
}

fn fuse_class(areas: &Areas, a: usize, b: usize) -> FuseClass {
    let (ra, ca) = rows_cols(areas, a, false);
    let (rb, cb) = rows_cols(areas, b, false);
    if ra.intersection(&rb).next().is_some() && ca.intersection(&cb).next().is_some() {
        return FuseClass::Both;
    }
    let (ra, ca) = rows_cols(areas, a, true);
    let (rb, cb) = rows_cols(areas, b, true);
    if ra.intersection(&rb).next().is_some() {
        FuseClass::Horiz
    } else if ca.intersection(&cb).next().is_some() {
        FuseClass::Vert
    } else {
        FuseClass::Angle
    }
}

/// Every fused pair of shaped areas with its connector class — the one
/// classification pass behind `axis_necks`, `circle_rect_necks` and the class-D
/// barrier list. Only fused pairs grew cell-adjacent (everyone else keeps a rock
/// gap), so one sweep over every cell's neighbours finds them all; `fuse_class`
/// then runs once per pair instead of once per pair per consumer. Pairs come out
/// sorted (`a < b`, ascending), the order the old per-consumer scans used.
///
/// The classification is on the PRE-claim room: `rows_cols` skips corridor floor, so
/// a pair's own corridor cannot reclassify the pair that produced it. The adjacency
/// sweep needs no such care — a fused pair already touched before its corridor
/// existed, and corridor floor may not touch a third area at all.
fn fuse_pairs(areas: &Areas) -> Vec<(usize, usize, FuseClass)> {
    let mut touching: std::collections::BTreeSet<(usize, usize)> =
        std::collections::BTreeSet::new();
    for a in 0..areas.count() {
        if areas.shape(a).is_none() {
            continue;
        }
        // ROOM cells, not floor cells. Fusion is two rooms growing into each other, and it is
        // their *rooms* that touch. Corridor floor is owned by an area but is not part of it, so a
        // pair joined by a claimed corridor has adjacent floor and non-adjacent rooms — reading
        // `floor_cells` here made every such pair look fused, and the two rooms were merged into
        // one compound with the wall between them cropped away.
        for h in areas.room_cells(a) {
            for nb in h.neighbors() {
                if areas.is_join(nb) {
                    continue;
                }
                if let Some(b) = areas
                    .owner_of(nb)
                    .filter(|&b| b != a && areas.shape(b).is_some())
                {
                    touching.insert((a.min(b), a.max(b)));
                }
            }
        }
    }
    touching
        .into_iter()
        .map(|(a, b)| (a, b, fuse_class(areas, a, b)))
        .collect()
}

/// Whether two shapes' drawn borders overlap. A class-D pair that overlaps is
/// already open once its internal barrier is cropped and needs no connector; one
/// that does not still wants a corridor across the gap.
fn shapes_overlap(a: &ruins::RuinShape, b: &ruins::RuinShape) -> bool {
    use ruins::RuinShape as R;
    match (*a, *b) {
        (
            R::Circle {
                cx: x1,
                cy: y1,
                r: r1,
            },
            R::Circle {
                cx: x2,
                cy: y2,
                r: r2,
            },
        ) => (x1 - x2).hypot(y1 - y2) < r1 + r2,
        (
            R::Rect {
                cx: x1,
                cy: y1,
                hw: w1,
                hh: h1,
            },
            R::Rect {
                cx: x2,
                cy: y2,
                hw: w2,
                hh: h2,
            },
        ) => (x1 - x2).abs() < w1 + w2 && (y1 - y2).abs() < h1 + h2,
        (
            R::Circle { cx, cy, r },
            R::Rect {
                cx: rx,
                cy: ry,
                hw,
                hh,
            },
        )
        | (
            R::Rect {
                cx: rx,
                cy: ry,
                hw,
                hh,
            },
            R::Circle { cx, cy, r },
        ) => {
            let dx = ((cx - rx).abs() - hw).max(0.0);
            let dy = ((cy - ry).abs() - hh).max(0.0);
            dx.hypot(dy) < r
        }
        _ => false,
    }
}
/// Snap a corridor wall offset onto the hex lattice, moving **inward** (`up` for a
/// lower bound, else a upper one).
///
/// The raw support clamp puts a wall wherever the two shapes' extents happen to
/// cross, and that is often a bad place for a wall: a circle's clamp bound *is* its
/// apex, so the wall leaves tangentially and the traced border wobbles across it;
/// elsewhere it slices cells in half and the claimed floor straddles it. Every wall
/// this generator already draws well — a rect's own edges — lies on a hex vertex
/// line, so the connector's walls go there too. The corridor gives up at most one
/// cell of width and gains a wall that parts whole cells.
///
/// Two cases, from the pointy-top hex's own symmetry:
/// - a **flat** normal (0°, ±60°, ±120° — the upright corridor and both diagonals):
///   a cell reaches exactly its apothem along `n`, and the lattice lines are apothem
///   multiples, so the wall lands on a cell's flat edge and the fit is **exact**.
/// - the lone **point** normal `(0,1)` (the level corridor): cells reach `s` but
///   their rows interlock every `1.5s`, so no line parts two rows cleanly. The wall
///   goes on a shoulder line, `1.5·s·r ± s/2` — exactly where a rect's top and
///   bottom edges go — and the outer row's points overhang it by `s/2`, just as a
///   rect's own cells do.
fn snap_wall(u: f64, n: Point, s: f64, up: bool) -> f64 {
    // A FLAT normal is left alone. There every apothem multiple is at once one
    // column's centre line and its neighbours' flat edges, so there is no
    // distinguished set of lines to snap to — a snapped wall bisects a column just
    // as readily as the raw clamp does, while giving up an apothem of width on each
    // side. Measured: snapping the flat normals as well costs 48 of 458 connectors,
    // most of them upright.
    if n.0.abs() > 1e-9 {
        return u;
    }
    // The point normal `(0,1)` does have such a set. Half-`s` multiples are the hex
    // vertex lines; drop the multiples of `1.5s`, which are row CENTRES and would
    // halve a row. What is left are the shoulder lines `1.5·s·r ± s/2`, exactly
    // where a rect's own top and bottom edges sit.
    let mut j = (if up {
        (u / (0.5 * s) - 1e-6).ceil()
    } else {
        (u / (0.5 * s) + 1e-6).floor()
    }) as i64;
    while j.rem_euclid(3) == 0 {
        j += if up { 1 } else { -1 };
    }
    j as f64 * 0.5 * s
}

/// Straight corridors along one axis, for **any** fused pair of geometric areas
/// (circle↔circle, rect↔rect, circle↔rect alike). One construction serves both
/// class **B** (`d` horizontal) and class **C** (`d` vertical) — see
/// plans/fuse-case-taxonomy.md.
///
/// The two walls sit at the **support clamp** along the normal `n` — the
/// intersection of both shapes' extents, `[max(lo), min(hi)]`. Taking the inner
/// bound on each side is what guarantees a wall still crosses *both* borders: any
/// wider and it passes clear of one. Each wall then runs parallel to `d`, from
/// where it meets the near shape's border to where it meets the far shape's.
///
/// Both bounds are then snapped inward onto the hex lattice — see [`snap_wall`] for
/// why a wall belongs on a cell edge rather than wherever the clamp happens to fall.
fn axis_necks(
    areas: &Areas,
    pairs: &[(usize, usize, FuseClass)],
    class: FuseClass,
    axes: &[(Point, Point)],
    s: f64,
) -> Vec<Neck> {
    let mut necks = Vec::new();
    for &(a, b, cls) in pairs {
        if cls != class {
            continue;
        }
        let (Some(sa), Some(sb)) = (areas.shape(a), areas.shape(b)) else {
            continue;
        };
        // A class-D pair whose shapes already overlap is open once cropped.
        if class == FuseClass::Both && shapes_overlap(&sa, &sb) {
            continue;
        }
        // Circle↔rectangle corner fusions belong to `circle_rect_necks` (the
        // hex-aligned angle neck); routing them here too would double the wall.
        if class == FuseClass::Angle
            && matches!(
                (&sa, &sb),
                (
                    ruins::RuinShape::Rect { .. },
                    ruins::RuinShape::Circle { .. }
                ) | (
                    ruins::RuinShape::Circle { .. },
                    ruins::RuinShape::Rect { .. }
                )
            )
        {
            continue;
        }
        // Pick the axis PER PAIR: the one where the two rooms touch across the widest run of
        // tiles. For the diagonals this is the mirror rule from the FIX diagrams — a pair
        // offset the other way needs the mirrored axis, and choosing by contact width selects
        // it automatically.
        //
        // The width IS the contact tiles' extent (`contact_span`): the ground the two rooms
        // actually share, each tile counted whole, so a wall placed at the result bounds a tile
        // rather than bisecting it. This replaced a clamp built from the fitted shapes' support
        // intervals, narrowed by a tangency guard and then widened back toward the contact tiles
        // anyway — three pixel reconciliations to arrive near the number the tiles give directly.
        // See `plans/tile-first-render.md` phase 3b.
        let Some((&(d, n), clamp_lo, clamp_hi)) = axes
            .iter()
            .filter_map(|ax @ &(_, n)| {
                let (lo, hi) = contact_span(areas, a, b, n, s)?;
                (hi - lo > 0.0).then_some((ax, lo, hi))
            })
            .max_by(|x, y| (x.2 - x.1).total_cmp(&(y.2 - y.1)))
        else {
            continue;
        };
        // The nearer shape along `d` faces forward (+1) and the farther back. The area each
        // belongs to travels with it, so a wall endpoint can fall back to that area's tiles.
        let ((sl, il), (sr, ir)) = if support(&sa, d) <= support(&sb, d) {
            ((sa, a), (sb, b))
        } else {
            ((sb, b), (sa, a))
        };
        // The full clamp is the widest corridor that still meets both borders,
        // and it is deliberately aggressive — that is what produces the
        // distinctive compound shapes. But a very long wall can cross other
        // geometry on the same loop. Rather than reject the pair outright,
        // emit a LADDER of candidates from the full clamp down; the splice
        // takes the first that fits, so an over-long corridor degrades into a
        // shorter one instead of vanishing.
        let (mid, half) = ((clamp_lo + clamp_hi) / 2.0, (clamp_hi - clamp_lo) / 2.0);
        let mut prev: Option<(f64, f64)> = None;
        // Nine rungs from the full clamp down to 0.36 of it. The step size is
        // load-bearing, not cosmetic: the same range in five coarser rungs finds
        // a fitting width far less often (measured: 446 connectors against 462),
        // because a wall that crosses something often clears it a couple of units
        // in, and a 20%-of-clamp step overshoots straight past that.
        // One candidate per pair, at the full contact width. This was a nine-rung ladder from
        // the full clamp down to 0.36 of it, so an over-long corridor could degrade into a
        // shorter one instead of being rejected. With the rejection gone the ladder has nothing
        // to feed, and it was costing quality as well as complexity: the narrower rungs put
        // walls in worse places (dense `worst` 1.7px with the ladder, 0.9px without).
        for scale in [1.0f64] {
            // Each rung snaps too, so no rung leaves a wall mid-row; two rungs
            // that land on the same pair of lines are one candidate.
            let u_lo = snap_wall(mid - half * scale, n, s, true);
            let u_hi = snap_wall(mid + half * scale, n, s, false);
            // Below a hex of opening it is a doorway, not a corridor.
            if u_hi - u_lo < s {
                break;
            }
            if prev == Some((u_lo, u_hi)) {
                continue;
            }
            prev = Some((u_lo, u_hi));
            // The wall at normal-offset `u`, as its two border endpoints.
            let at = |u: f64, t: f64| (n.0 * u + d.0 * t, n.1 * u + d.1 * t);
            // A wall ends on the room's own border where that border crosses the line, and on
            // the room's outermost TILE edge where it does not — the tile-clamp above reaches
            // rows the fitted shape falls short of, and those walls need somewhere to end.
            let end = |sh: &ruins::RuinShape, i: usize, sgn: f64, u: f64| -> Option<f64> {
                border_along(sh, n, u, d, sgn)
                    .or_else(|| tile_border_along(areas, i, n, u, d, sgn, s))
            };
            let wall = |u: f64| -> Option<(Point, Point)> {
                Some((at(u, end(&sl, il, 1.0, u)?), at(u, end(&sr, ir, -1.0, u)?)))
            };
            let (Some(top), Some(bot)) = (wall(u_lo), wall(u_hi)) else {
                continue;
            };
            // The connector IS its two walls, so carry them: each runs from the near
            // room's border to the far room's, and a curved border meets the two at
            // different points, so they are generally unequal. A rectangle between their
            // midpoints was only as long as their average — the longer wall's end fell
            // outside its own footprint, and the shorter one reached into a room.
            let hall = ruins::RuinShape::Trapezoid {
                wall0: top,
                wall1: bot,
            };
            necks.push(Neck {
                pair: (a, b),
                shape_a: sl,
                shape_b: sr,
                lines: [top, bot],
                hall,
                kind: NeckKind::AxisCorridor,
                claimed: Vec::new(),
                cell: s,
            });
        }
    }
    necks
}

/// Class-A angle necks, for fused circle↔rectangle corner pairs (see
/// [`splice_necks`]).
fn circle_rect_necks(areas: &Areas, pairs: &[(usize, usize, FuseClass)], s: f64) -> Vec<Neck> {
    use ruins::RuinShape;
    let mut necks = Vec::new();
    // Only fused CORNER pairs get the angle neck; the other classes have their
    // own constructions (see `fuse_class`). Reordered rectangle-first, and
    // emitted in (rectangle, circle) index order.
    let mut rc_pairs: Vec<(usize, usize)> = pairs
        .iter()
        .filter(|&&(_, _, cls)| cls == FuseClass::Angle)
        .filter_map(|&(i, j, _)| match (areas.shape(i), areas.shape(j)) {
            (Some(RuinShape::Rect { .. }), Some(RuinShape::Circle { .. })) => Some((i, j)),
            (Some(RuinShape::Circle { .. }), Some(RuinShape::Rect { .. })) => Some((j, i)),
            _ => None,
        })
        .collect();
    rc_pairs.sort_unstable();
    for (a, b) in rc_pairs {
        // a = rectangle, b = circle.
        let (
            Some(
                rect @ RuinShape::Rect {
                    cx: rcx,
                    cy: rcy,
                    hw: rhw,
                    hh: rhh,
                },
            ),
            Some(
                circ @ RuinShape::Circle {
                    cx: ccx,
                    cy: ccy,
                    r: cr,
                },
            ),
        ) = (areas.shape(a), areas.shape(b))
        else {
            continue;
        };
        // Which side the circle lies on, and the rectangle's near edge x.
        let sgnx = if ccx < rcx { -1.0 } else { 1.0 };
        let near_x = rcx + sgnx * rhw;
        // Extend anchor `p` along `dir` to the circle's outer arc (nearest).
        let hit = |p: Point, dir: (f64, f64)| -> Option<Point> {
            let (ex, ey) = (p.0 - ccx, p.1 - ccy);
            let bq = ex * dir.0 + ey * dir.1;
            let cq = ex * ex + ey * ey - cr * cr;
            let disc = bq * bq - cq;
            (disc >= 0.0)
                .then(|| -bq - disc.sqrt())
                .filter(|&t| t > 0.0)
                .map(|t| (p.0 + t * dir.0, p.1 + t * dir.1))
        };
        // The angle geometry below assumes the circle sits beyond a
        // left/right edge (x-dominant offset). A corner beyond a top/bottom
        // edge needs the transposed construction — deferred.
        if ((ccx - rcx) / rhw).abs() < ((ccy - rcy) / rhh).abs() {
            continue;
        }
        let Some(conn) = areas
            .floor_cells(a)
            .filter(|h| {
                h.neighbors()
                    .iter()
                    .any(|nb| areas.owner_of(*nb) == Some(b))
            })
            .min_by(|p, q| {
                (p.center(s).0 - near_x)
                    .abs()
                    .total_cmp(&(q.center(s).0 - near_x).abs())
            })
        else {
            continue;
        };
        let sgny = if ccy < rcy { -1.0 } else { 1.0 };
        let cc = conn.center(s);
        // Anchors on the rectangle's near edge: the near corner on the
        // circle's vertical side, and the connecting hex's opposite point.
        let corner = (near_x, rcy + sgny * rhh);
        let hex_pt = (cc.0, cc.1 - sgny * s);
        // Neck direction: the hex-edge diagonal pointing at the circle.
        let dir = (sgnx * grid::HEX_APOTHEM, sgny * 0.5);
        let (Some(c_hit), Some(h_hit)) = (hit(corner, dir), hit(hex_pt, dir)) else {
            continue;
        };
        // A `StraightHall` whose two sides are the neck walls: centreline
        // midway between them, half-width the perpendicular half-distance.
        let mid0 = ((corner.0 + hex_pt.0) / 2.0, (corner.1 + hex_pt.1) / 2.0);
        let mid1 = ((c_hit.0 + h_hit.0) / 2.0, (c_hit.1 + h_hit.1) / 2.0);
        let nrm = (-dir.1, dir.0);
        let hw = ((corner.0 - hex_pt.0) * nrm.0 + (corner.1 - hex_pt.1) * nrm.1).abs() / 2.0;
        let hall = RuinShape::StraightHall {
            ax: mid0.0,
            ay: mid0.1,
            bx: mid1.0,
            by: mid1.1,
            hw,
        };
        necks.push(Neck {
            pair: (a, b),
            shape_a: rect,
            shape_b: circ,
            lines: [(c_hit, corner), (h_hit, hex_pt)],
            hall,
            kind: NeckKind::AngleNeck,
            claimed: Vec::new(),
            cell: s,
        });
    }
    necks
}

/// One step of a connector splice: what [`Neck::splice_walk`] tells its caller
/// to emit next.
enum SpliceStep {
    /// Sequence vertex `i` lies outside the connector — copy it through.
    Keep(usize),
    /// A dropped run was replaced by this stretch of wall, in traversal order;
    /// each endpoint is flagged whether it is the wall's own end (on a room
    /// border) rather than a doorway jamb the clip stopped at.
    Wall((Point, bool), (Point, bool)),
}

impl Neck {
    /// Walk one vertex sequence and replace every contiguous run of footprint
    /// vertices with its clipped wall stretch — the ONE traversal behind both
    /// splice layers ([`splice_necks`] for the wall band, [`spliced_loop`] for
    /// the floor outline), so run clipping can never drift between them.
    ///
    /// `n`/`pos` describe the sequence WITHOUT the repeated closing vertex;
    /// `closed` says whether it wraps. `run_end` resolves the `next` neighbour
    /// when an OPEN sequence ends inside the connector — the wall band is cut at
    /// every doorway gap, so it asks the doorway for its jamb; a closed floor
    /// loop never reaches it. Returns `false` (nothing emitted) when every
    /// vertex is inside the connector, which callers treat as "leave the
    /// sequence alone".
    fn splice_walk(
        &self,
        n: usize,
        closed: bool,
        pos: &dyn Fn(usize) -> Point,
        run_end: &dyn Fn((Point, Point), Point) -> Point,
        emit: &mut dyn FnMut(SpliceStep),
    ) -> bool {
        let Some(f) = self.frame() else { return false };
        // Signed perpendicular offset from the hall centreline: which side a
        // vertex (or a wall line's midpoint) lies on.
        let pp = |p: Point| f.local(p).1;
        let side0 = pp((
            (self.lines[0].0.0 + self.lines[0].1.0) / 2.0,
            (self.lines[0].0.1 + self.lines[0].1.1) / 2.0,
        ))
        .signum();
        // A CLOSED sequence rotates to start on a kept vertex, else a dropped run
        // wrapping the seam would be split in two. An OPEN one must NOT rotate: it
        // is a polyline, not a cycle, so reordering it welds parts that are not
        // neighbours. (A band run is cut open at every doorway gap; rotating one
        // drew a wall straight across a room's interior, from the far end of its
        // arc back to the spliced stretch.) Its own ends are gap edges instead, so
        // `run_end` resolves both.
        let Some(first_kept) = (0..n).find(|&i| !self.blocks(pos(i))) else {
            return false;
        };
        let idx = |k: usize| if closed { (first_kept + k) % n } else { k };
        // The last position emitted, for orienting each wall stretch.
        let mut last: Option<Point> = None;
        let mut k = 0;
        while k < n {
            let i = idx(k);
            let v = pos(i);
            if self.blocks(v) {
                let li = if side0 == pp(v).signum() { 0 } else { 1 };
                let line = self.lines[li];
                let first_dropped = v;
                let mut last_dropped = v;
                while k < n && self.blocks(pos(idx(k))) {
                    last_dropped = pos(idx(k));
                    k += 1;
                }
                // Clip the wall to the stretch this run covered, so a doorway
                // interrupting the run leaves a gap instead of a spike.
                let prev = last.unwrap_or_else(|| run_end(line, first_dropped));
                let next = if k < n {
                    pos(idx(k))
                } else if closed {
                    pos(idx(0))
                } else {
                    run_end(line, last_dropped)
                };
                let ((e0, at0), (e1, at1)) = self.wall_stretch(line, prev, next);
                emit(SpliceStep::Wall((e0, at0), (e1, at1)));
                last = Some(e1);
            } else {
                emit(SpliceStep::Keep(i));
                last = Some(v);
                k += 1;
            }
        }
        true
    }
}

/// Splice each [`Neck`] into the `dungeon_walls` band: in the merged compound
/// run, the two seam crossings (where circle vertices meet rectangle vertices,
/// the pinch) are replaced by the neck's outer wall line for that side, tagged
/// with the hall so the renderer offsets its inner wall. The band then flows
/// circle arc → neck → rectangle wall as one continuous wall.
fn splice_necks(walls: &mut [WallRun], necks: &[Neck], doors: &[Point]) {
    for neck in necks.iter().filter(|n| !n.is_link()) {
        // Which of the two shapes a spliced endpoint sits on — the endpoint is on
        // one border or the other by construction, so the nearer one wins.
        let shape_at = |p: Point| -> ruins::RuinShape {
            if neck.shape_a.wall_dist(p) <= neck.shape_b.wall_dist(p) {
                neck.shape_a
            } else {
                neck.shape_b
            }
        };
        for run in walls.iter_mut() {
            // The run must be one of this pair's and must actually pass through the
            // connector. Requiring *both* shapes in one run was too strict: a
            // doorway crossing the corridor splits the band, so the two rooms can
            // land in separate runs and neither would get the corridor's wall —
            // which is how a connector ended up with walls in the floor outline but
            // none in the band. Per-run clipping (see `wall_stretch`) makes the
            // weaker test safe: each run only ever receives its own stretch.
            if !run
                .iter()
                .any(|v| v.1 == neck.shape_a || v.1 == neck.shape_b)
                || !run.iter().any(|v| neck.blocks(v.0))
            {
                continue;
            }
            let closed = run.len() > 2 && run.first().map(|v| v.0) == run.last().map(|v| v.0);
            let core = if closed {
                &run[..run.len() - 1]
            } else {
                &run[..]
            };
            let mut out: WallRun = Vec::with_capacity(core.len() + 4);
            let ok = neck.splice_walk(
                core.len(),
                closed,
                &|i| core[i].0,
                // The band ran off the end of an open run, i.e. into a doorway
                // gap: ask the doorway where its jamb is, since the band's own
                // last vertex stops a cell short of it.
                &|line, last_dropped| {
                    neck.jamb_edge(line, doors, last_dropped)
                        .unwrap_or(last_dropped)
                },
                &mut |step| match step {
                    SpliceStep::Keep(i) => out.push(core[i]),
                    SpliceStep::Wall((e0, at0), (e1, at1)) => {
                        if neck.is_corridor() {
                            // Wide corridor. Both wall endpoints are hall-tagged
                            // (their inner offset drops straight to the corridor
                            // half-width → a straight inner line). An endpoint on a
                            // room border is emitted as a COINCIDENT pair with that
                            // room's shape, so the renderer mitres the inner corner
                            // where the corridor wall meets the room's wall
                            // (straight for a rect edge, curving for an arc); the
                            // matching-room tag sits against that room's own
                            // vertices — first at `e0` (predecessor), last at `e1`
                            // (successor). An end the clip pulled *inside* the wall
                            // is a doorway jamb, not a room border, so it stays
                            // hall-tagged — a room shape there would drag the jamb
                            // onto that room's wall.
                            if at0 {
                                out.push((e0, shape_at(e0)));
                            }
                            out.push((e0, neck.hall));
                            out.push((e1, neck.hall));
                            if at1 {
                                out.push((e1, shape_at(e1)));
                            }
                        } else {
                            out.push((e0, neck.hall));
                            out.push((e1, neck.hall));
                        }
                    }
                },
            );
            if !ok {
                continue;
            }
            if closed && let Some(&first) = out.first() {
                out.push(first);
            }
            *run = out;
        }
    }
}

/// Splice one connector into a single floor-outline loop: replace each
/// contiguous run of footprint vertices with its wall stretch, via the same
/// [`Neck::splice_walk`] traversal the wall band uses. Returns the rebuilt loop
/// plus the indices of the inserted wall endpoints (for the safety check), or
/// `None` if this loop doesn't pass through the connector (leave it unchanged).
fn spliced_loop(neck: &Neck, loop_: &[Point]) -> Option<(Vec<Point>, Vec<usize>)> {
    if !loop_.iter().any(|&p| neck.blocks(p)) {
        return None;
    }
    let closed = loop_.len() > 2 && loop_.first() == loop_.last();
    let core = if closed {
        &loop_[..loop_.len() - 1]
    } else {
        loop_
    };
    let mut out: Vec<Point> = Vec::with_capacity(core.len() + 2);
    let mut inserted: Vec<usize> = Vec::new();
    let ok = neck.splice_walk(
        core.len(),
        closed,
        &|i| core[i],
        // Floor loops are closed, so an open run-end never resolves here; the
        // run's own outermost dropped vertex is the (unreachable) fallback.
        &|_line, last_dropped| last_dropped,
        &mut |step| match step {
            SpliceStep::Keep(i) => out.push(core[i]),
            SpliceStep::Wall((e0, _), (e1, _)) => {
                inserted.push(out.len());
                out.push(e0);
                inserted.push(out.len());
                out.push(e1);
            }
        },
    );
    if !ok {
        return None;
    }
    if closed && let Some(&first) = out.first() {
        out.push(first);
    }
    Some((out, inserted))
}

/// Splice each level-corridor connector's outer walls into the floor outline so
/// the `#fp` border and fill follow the corridor rather than the old pinched
/// cell-union seam (splicing only the wall band would leave a stray outline line
/// across the opening).
///
/// Returns the connectors to splice into the wall band — every wide corridor
/// that was actually accepted here, plus all the angle necks untouched — so
/// the two layers never disagree about whether a corridor exists.
///
/// Each candidate is applied **cumulatively** and rolled back if it folds a loop:
/// where the fusion is not a clean side-by-side, a border can double back across
/// the corridor wall, and two connectors that are each fine alone can still
/// conflict once both are in. A rejected connector falls back to the band merge.
fn splice_outline_necks(outline: &mut [Vec<Point>], necks: Vec<Neck>) -> Vec<Neck> {
    let mut kept: Vec<Neck> = Vec::with_capacity(necks.len());
    let mut placed: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for neck in necks {
        if !neck.is_corridor() {
            kept.push(neck);
            continue;
        }
        // Candidates are widest-first, so once a pair is placed the narrower
        // fallbacks for it are moot.
        if placed.contains(&neck.pair) {
            continue;
        }
        // Spliced unconditionally. There used to be a `inserted_walls_cross` gate here that
        // rejected a corridor whose inserted walls crossed the rest of the loop, and an
        // acceptance ladder feeding it narrower and narrower candidates until one passed. Both
        // were a RENDER constraint deciding whether a corridor exists: growth had already
        // claimed the tiles, and the only thing that may refuse a claim is the rock gap to a
        // non-fused area. A wall that crosses a room is now clipped away by
        // `clip::split_outside` instead of preventing the corridor — measured, that is both
        // more corridors (355 -> 731 dense) and shallower walls (1.7px -> 0.9px).
        let proposed: Vec<(usize, Vec<Point>)> = outline
            .iter()
            .enumerate()
            .filter_map(|(li, lp)| spliced_loop(&neck, lp).map(|(new, _)| (li, new)))
            .collect();
        if !proposed.is_empty() {
            for (li, new) in proposed {
                outline[li] = new;
            }
            placed.insert(neck.pair);
            kept.push(neck);
        }
    }
    kept
}

/// Trim a corridor's two walls to the stretch that lies **outside both rooms**.
///
/// A corridor is built from the tiles the two rooms share, so each of its walls starts deep
/// inside one room, crosses whatever gap there is, and ends deep inside the other. Only the
/// middle is wall: the ends are lines drawn across open room floor.
///
/// [`clip::split_outside`](crate::clip::split_outside) already removes those ends from the
/// rendered wall band, but the **outline** splice inserts the untrimmed wall endpoints straight
/// into the floor loop, so the loop dives into one room's interior and back out — which is what
/// folds it. Trimming at source fixes both layers from one place and keeps them agreeing, which
/// splicing one and clipping the other cannot.
///
/// `false` when a wall has no outside stretch at all: the two borders already overlap, so the
/// compound is open without a corridor and the pair needs none (the `crop`/`square_seams` path
/// handles it). The caller drops the candidate.
fn trim_to_gap(neck: &mut Neck, rooms: &[ruins::RuinShape]) -> bool {
    // The longest run of `a`→`b` that no room encloses, as (start, end) points.
    let longest_outside = |a: Point, b: Point| -> Option<(Point, Point)> {
        let at = |t: f64| {
            if t <= 0.0 {
                a
            } else if t >= 1.0 {
                b
            } else {
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            }
        };
        let mut ts: Vec<f64> = vec![0.0, 1.0];
        for r in rooms {
            ts.extend(r.segment_crossings(a, b));
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        // Between consecutive breakpoints the segment is wholly in or wholly out, so one
        // midpoint decides the interval. Keep the longest outside one: a corridor spans one
        // gap, and a shorter sliver is a graze past a corner.
        ts.windows(2)
            .filter(|w| {
                let m = at(0.5 * (w[0] + w[1]));
                !rooms.iter().any(|r| r.contains(m))
            })
            .max_by(|x, y| (x[1] - x[0]).total_cmp(&(y[1] - y[0])))
            .map(|w| (at(w[0]), at(w[1])))
    };
    let (Some(w0), Some(w1)) = (
        longest_outside(neck.lines[0].0, neck.lines[0].1),
        longest_outside(neck.lines[1].0, neck.lines[1].1),
    ) else {
        return false;
    };
    neck.lines = [w0, w1];
    neck.hall = ruins::RuinShape::Trapezoid {
        wall0: w0,
        wall1: w1,
    };
    true
}

/// Every connector candidate for every fused pair, widest-first per pair.
///
/// Returns the candidates and the pair classification they were built from, so a caller
/// wanting both does not re-run [`fuse_pairs`].
///
/// One classification pass feeds all four axis families plus the angle neck. Called
/// twice per map — once by growth, to find where a corridor wants floor, and once after
/// the outline to draw the walls — and the necks are **stale** by the second call, which
/// is why they are re-derived rather than carried. Between the two: `finalize` and
/// `keep_largest_component` drop areas and re-index, `topology::build` shrinks ruin
/// areas, and `ruins::build` erodes ruin room boundaries and can refit or demote one to
/// no shape at all. The walls must be drawn against the geometry that survived all that.
fn plan_necks(
    areas: &Areas,
    connections: &[crate::topology::Connection],
    s: f64,
) -> (Vec<Neck>, Vec<(usize, usize, FuseClass)>) {
    // The two hex diagonals: corridor axis ∓30°, normal 60°/120°. Which one a
    // pair uses is chosen per pair inside `axis_necks` (the mirror rule).
    const AP: f64 = grid::HEX_APOTHEM;
    const DIAGONALS: [(Point, Point); 2] = [((AP, -0.5), (0.5, AP)), ((AP, 0.5), (-0.5, AP))];
    const LEVEL: [(Point, Point); 1] = [((1.0, 0.0), (0.0, 1.0))];
    const UPRIGHT: [(Point, Point); 1] = [((0.0, 1.0), (1.0, 0.0))];
    // Class A gets both axis-aligned axes and picks per pair: these corner pairs
    // are geometrically near-stacked (overlapping on one axis with a small gap on
    // the other), so the axis clamp already lands the walls right and the "L" is
    // just the junction with each room's own edge, which the outline traces anyway.
    const BOTH_AXES: [(Point, Point); 2] = [((1.0, 0.0), (0.0, 1.0)), ((0.0, 1.0), (1.0, 0.0))];

    let pairs = fuse_pairs(areas);
    let mut necks = axis_necks(areas, &pairs, FuseClass::Horiz, &LEVEL, s);
    necks.extend(axis_necks(areas, &pairs, FuseClass::Vert, &UPRIGHT, s));
    // Class D pairs whose shapes do NOT overlap: a diagonal corridor across the gap.
    necks.extend(axis_necks(areas, &pairs, FuseClass::Both, &DIAGONALS, s));
    necks.extend(axis_necks(areas, &pairs, FuseClass::Angle, &BOTH_AXES, s));
    necks.extend(circle_rect_necks(areas, &pairs, s));
    // Last: a tile-only corridor for any fused pair the constructions above could not build
    // one for, so a corridor exists between every fused pair.
    // Every corridor is trimmed to the gap it actually spans, so the outline splice and the
    // wall band insert the same geometry and neither carries a line across open room floor.
    let rooms: Vec<ruins::RuinShape> = (0..areas.count())
        .filter_map(|i| areas.shape(i))
        .filter(|sh| {
            matches!(
                sh,
                ruins::RuinShape::Circle { .. } | ruins::RuinShape::Rect { .. }
            )
        })
        .collect();
    necks.retain_mut(|n| !n.is_corridor() || trim_to_gap(n, &rooms));
    // Room-to-room links, from the floor `topology` reserved for each. A different population from
    // the fused pairs above — `candidate_cells_by_pair` skips intra-group pairs — so the two never
    // compete for the same pair and need no coverage negotiation between them.
    //
    // Trimmed where possible but never DROPPED. `trim_to_gap` declines a corridor whose walls lie
    // wholly inside the two rooms, on the reasoning that the compound is already open and needs
    // none — true of a fused pair whose borders overlap, false of a connection, whose floor was
    // deliberately reserved and must be walled. `clip::split_outside` removes whatever stretch does
    // lie inside a room, so keeping it costs nothing.
    let mut links = connection_necks(areas, connections, s);
    for n in links.iter_mut() {
        trim_to_gap(n, &rooms);
    }
    necks.extend(links);
    // `contact_necks` runs AFTER the trim, and `covered` is read off the survivors. Taking it
    // before meant a pair whose only candidate trimmed away counted as served, so the fallback
    // never fired and the pair ended with no corridor at all — its claimed floor left unwalled.
    // The last-resort construction is only a guarantee if it sees what actually survived.
    let covered: Vec<(usize, usize)> = necks.iter().map(|n| n.pair).collect();
    let mut extra = contact_necks(areas, &pairs, &covered, s);
    // Trimmed if it can be, but NOT dropped if it cannot. `trim_to_gap` declines a corridor whose
    // walls lie wholly inside the two rooms, reasoning that the compound is already open and needs
    // none — true of a fused pair whose borders overlap, false of a pair whose floor growth
    // claimed across a real gap. This is the last resort: dropping it here leaves that floor with
    // no wall at all, whereas keeping it untrimmed costs nothing, since `clip::split_outside`
    // removes whatever stretch does lie inside a room.
    for n in extra.iter_mut() {
        trim_to_gap(n, &rooms);
    }
    necks.extend(extra);
    (necks, pairs)
}

/// `p`'s offset ACROSS the corridor if it lies in the span that corridor claims floor
/// for — between the two rooms' borders along the axis, and STRICTLY between the two
/// walls across it — else `None`.
///
/// The one definition, shared by `corridor_floor` (which asks growth for those cells)
/// and `plan` (which recovers them afterwards to build each candidate's footprint).
/// They must agree exactly: a claimed cell missing from the footprint keeps its
/// vertices through the splice, and they are then floor poking past a wall.
///
/// Returns the offset rather than a bool because `corridor_floor` groups cells by it, and
/// it is the expensive half of the test — recomputing it cost a second `Frame::local` per
/// grid cell per corridor.
fn claim_offset(neck: &Neck, p: Point) -> Option<f64> {
    let (f, hw) = (neck.frame()?, neck.half_width()?);
    span_offset(neck, &f, hw, p)
}

/// [`claim_offset`] with the frame and half-width already in hand — the form the per-cell
/// loop wants, since rebuilding a `Frame` is a `hypot` and four divides.
fn span_offset(neck: &Neck, f: &Frame, hw: f64, p: Point) -> Option<f64> {
    let (t, pp) = f.local(p);
    if pp.abs() >= hw - 1e-6 {
        return None;
    }
    // The along-bounds follow the trapezoid (see `Neck::span_at`), so a cell past the
    // corridor's *average* length still counts where the wall beside it actually reaches.
    let (near, far) = neck.span_at(f, pp);
    (t >= near.min(far) - FOOTPRINT_SLOP && t <= near.max(far) + FOOTPRINT_SLOP).then_some(pp)
}

/// One side of one corridor, as the lattice **lines** running along it, ordered
/// outward from the centreline: `growth` walks them in order and stops at the first
/// it cannot complete.
///
/// Cells sharing an offset across the corridor share a line, whatever the axis, so
/// one grouping serves level, upright and diagonal alike (a level corridor's lines
/// are hex rows; an upright corridor's are the true vertical columns, whose cells
/// sit two apart — see `rows_cols`). Each cell carries which of the pair should own
/// it: floor beside the circle belongs to the circle's room.
pub(crate) struct CorridorSide {
    pub(crate) pair: (usize, usize),
    pub(crate) lines: Vec<Vec<(grid::Hex, usize)>>,
}

/// Where every fused pair's corridor wants floor, as lines to walk outward.
///
/// WHY a corridor needs floor at all: a connector redraws walls and the floor
/// *outline* but claims no cells of its own. On a wide fusion that is invisible — the
/// floor already spans the contact — and on a narrow one it is the whole problem: a
/// pair touching across a single hex face has one hex of floor under a corridor
/// several hexes long, so the corridor's walls run through solid rock and cross the
/// real floor boundary. The acceptance ladder then degrades the corridor to a stub
/// (the repro pair took the 0.36 rung, 25 units of a 70-unit clamp). Filling the span
/// is what lets the full clamp stand.
///
/// This function is geometry only — it says nothing about whether a cell may be
/// taken. That is [`growth::claim_join_floor`](crate::growth)'s call, which is why
/// the safety rule lives there and in one place; this function used to claim the
/// cells itself and mirror that rule by hand.
///
/// Only the wide corridors ask: the narrow hex-aligned angle neck already runs
/// along cells that are floor. The widest candidate per pair asks (candidates are
/// widest-first), because the floor has to cover the widest wall the ladder might
/// accept.
pub(crate) fn corridor_floor(areas: &Areas, grid: &HexGrid, s: f64) -> Vec<CorridorSide> {
    use std::collections::BTreeMap;
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    // No connections yet: this runs from `growth`, before `topology` chooses any edge.
    for neck in plan_necks(areas, &[], s).0 {
        let (a, b) = neck.pair;
        if !neck.is_corridor() || !seen.insert((a, b)) {
            continue;
        }
        // Dungeon pairs only. Corridor floor exists to give a connector's walls something
        // to enclose, so claiming it for a pair that will never get a connector leaves floor
        // no wall covers — which is what `sweep`'s `fo` counts. Ruin fusion is still
        // deferred, so a fused ruin pair reaches the ladder and every candidate is rejected;
        // the floor then survives on the late rollback in `commit_join_floor` alone, and it
        // does not always catch it. Not claiming is the same rule the disk ring follows in
        // `growth::Shape::candidates`: do not take ground you cannot finish using.
        //
        // Measured: connectors are untouched at 120 seeds (39 at tag defaults, 482 and 442
        // dense — identical), and `fo` goes 1 -> 0. Ruin pairs were contributing no accepted
        // connector at all, only the orphaned floor.
        if areas.kind(a) != AreaKind::Dungeon || areas.kind(b) != AreaKind::Dungeon {
            continue;
        }
        let (Some(sa), Some(sb)) = (areas.shape(a), areas.shape(b)) else {
            continue;
        };
        let (Some(f), Some(hw)) = (neck.frame(), neck.half_width()) else {
            continue;
        };
        // A corridor covers a handful of cells out of the whole grid (measured: 0.4%), so
        // reject on the hall's bounding box — inflated by the half-width and the footprint
        // slop, i.e. everything `span_offset` could still admit — before any frame work.
        let end = (f.o.0 + f.dir.0 * f.len, f.o.1 + f.dir.1 * f.len);
        let pad = hw + FOOTPRINT_SLOP;
        let (x0, x1) = (f.o.0.min(end.0) - pad, f.o.0.max(end.0) + pad);
        let (y0, y1) = (f.o.1.min(end.1) - pad, f.o.1.max(end.1) + pad);
        // Group the cells the corridor covers into lines by their offset across it.
        let mut lines: BTreeMap<i64, Vec<(grid::Hex, usize)>> = BTreeMap::new();
        for &c in grid.cells() {
            let p = c.center(s);
            if p.0 < x0 || p.0 > x1 || p.1 < y0 || p.1 > y1 {
                continue;
            }
            // Between the two rooms' borders along the axis — the ends are the
            // borders themselves, where the floor already is — and STRICTLY
            // between the two walls across it. A cell centred *on* a wall is half
            // outside it, and that is the common case rather than a fluke: the
            // clamp bounds are the rooms' own hex-aligned borders, which land on
            // cell centres.
            let Some(pp) = span_offset(&neck, &f, hw, p) else {
                continue;
            };
            let owner = if sa.wall_dist(p) <= sb.wall_dist(p) {
                a
            } else {
                b
            };
            lines
                .entry((pp * 64.0).round() as i64)
                .or_default()
                .push((c, owner));
        }
        // Split into the two sides and order each outward from the centreline.
        let keys: Vec<i64> = lines.keys().copied().collect();
        let split = keys.partition_point(|&k| k < 0);
        for side in [
            keys[split..].to_vec(),
            keys[..split].iter().rev().copied().collect::<Vec<_>>(),
        ] {
            // Each key belongs to exactly one side, so the lines can be moved out.
            let ls: Vec<Vec<(grid::Hex, usize)>> =
                side.into_iter().filter_map(|k| lines.remove(&k)).collect();
            if !ls.is_empty() {
                out.push(CorridorSide {
                    pair: (a, b),
                    lines: ls,
                });
            }
        }
    }
    out
}

/// Commit growth's corridor floor, now that the ladder has settled which connectors are
/// real: a join cell stays exactly when some wall encloses it.
///
/// Growth cannot decide this — whether a corridor gets a wall depends on the acceptance
/// ladder, which needs the traced outline — so the floor is laid first (`topology` and
/// `ruins` both have to see a corridor as floor) and confirmed here, at the first point
/// the answer exists. Everything downstream that reads the cell set (water, stones, the
/// floor pattern) runs after this.
///
/// Two ways a claimed cell ends up enclosed by nothing, both needing the ladder outcome:
///
/// 1. **Flank overhang.** Growth claims for the full clamp, but the ladder may settle on
///    a narrower rung, leaving the outermost cells beyond that rung's side walls (96
///    cells over the measured sweep, by up to one hex). The floor *outline* is right
///    either way here — an accepted neck exists, and its footprint covers every cell it
///    claimed whichever rung it ends on, so those vertices are spliced away regardless.
/// 2. **Rejected connector.** No rung was accepted, so fusion draws the pair no wall at
///    all, and whatever the rooms did not grow over is a lobe outside both. Note the
///    limit: there is no accepted neck, so nothing splices the outline, and the traced
///    boundary keeps the lobe even though the cell set no longer calls it floor. Fixing
///    that would mean re-tracing after the ladder, which is a larger change than the
///    stray is worth; the metric that watches this (`sweep.rs`'s `fo`) reads the cell set.
///
/// There is no longer a third way. Floor whose pair dissolved entirely used to be released
/// early, before the outline was traced, by a `release_orphans` backstop. Every case it
/// existed for is now prevented: erosion anchors join floor (`ruins::erode`), and a cell where
/// three areas meet is its own section with its own walls ([`junction_cells`]). Measured at 0
/// released cells over 3000 maps, so the backstop and the bookkeeping it needed are gone.
///
/// A corridor **mouth** survives: the hall's containment test clamps to the segment, so a
/// cell overhanging an end still counts as inside if it is within the half-width of the
/// end cap, and one sitting against a room's border is inside that room. Dropping those
/// would punch a hole in the mouth.
fn commit_join_floor(areas: &mut Areas, walls: &[WallRun], s: f64) {
    // Tested against the walls that were actually DRAWN, not against the necks that were kept.
    // Keeping a neck is not the same as drawing its wall: `splice_necks` filters links out, and
    // `clip::split_outside` can clip a run away entirely, so the kept set overestimates what is
    // enclosed. Both a neck's `hall` and its claim window (`claim_offset`) are intermediate
    // decisions; the wall layer is the artifact, and this function's job is to keep only floor a
    // wall encloses.
    let mut enclosures: Vec<ruins::RuinShape> = areas.shapes().iter().flatten().copied().collect();
    enclosures.extend(walls.iter().flatten().map(|&(_, sh)| sh));
    // A run tags every vertex with its shape, so the extend pushes ~6x consecutive duplicates
    // (measured: 403 -> 68 shapes at the densest tags). Consecutive by construction, so `dedup`
    // is enough.
    enclosures.dedup();
    // Collected before mutating: `areas.join()` borrows `areas`, `remove_from_area` needs
    // it mutably. Grouped by area so each pays one `retain` rather than one per cell.
    let mut by_area: std::collections::BTreeMap<usize, Vec<grid::Hex>> =
        std::collections::BTreeMap::new();
    for &c in areas.join() {
        // `join` is pruned on removal, so every cell in it is owned floor.
        let Some(i) = areas.owner_of(c) else { continue };
        let p = c.center(s);
        if !enclosures.iter().any(|sh| sh.contains(p)) {
            by_area.entry(i).or_default().push(c);
        }
    }
    for (i, cells) in by_area {
        areas.remove_from_area(i, &cells);
    }
}

// ---------------------------------------------------------------------------
// The two entry points `generate_with` uses.
// ---------------------------------------------------------------------------

/// Everything the fusion pass decided, carried from [`plan`] to
/// [`Fusion::apply`]. The split is forced by ordering, not preference: connector
/// geometry has to exist *before* door placement, so mouths land on the
/// post-fusion wall rather than on a barrier the splice later replaces; but
/// accepting a connector needs the traced floor outline, which needs jambs, which
/// needs those mouths.
pub(crate) struct Fusion {
    /// Candidates, widest-first per pair. Door placement sees this unfiltered
    /// set, which is harmless: avoiding a connector that is later dropped only
    /// nudges a door slightly.
    necks: Vec<Neck>,
    cell: f64,
}

/// Classify every fused pair, build its connector candidates, and recover the corridor
/// floor growth claimed for each. Reads `areas`; mutates nothing.
///
/// Returns the plan plus the cells the outline must lock on their own hex corners — the
/// narrow seams' and the claimed corridor floor's. Both belong to the join rather than
/// to either room, so projecting them onto a room wall would pinch the seam shut or
/// undo the fill.
pub(crate) fn plan(
    areas: &Areas,
    topology: &Topology,
    s: f64,
) -> (Fusion, std::collections::HashSet<grid::Hex>) {
    let (mut necks, _pairs) = plan_necks(areas, &topology.connections, s);
    // Growth already claimed the corridor floor (`growth::claim_join_floor`); recover
    // which cells serve which pair from the geometry, so each candidate knows the
    // footprint it must cover. Every candidate of a pair gets the same footprint, so the
    // span is computed once per pair and shared.
    let mut spans: std::collections::HashMap<(usize, usize), Vec<Point>> =
        std::collections::HashMap::new();
    for n in necks.iter().filter(|n| n.is_corridor()) {
        if let std::collections::hash_map::Entry::Vacant(slot) = spans.entry(n.pair) {
            let mine: Vec<grid::Hex> = areas
                .join()
                .iter()
                .copied()
                .filter(|c| claim_offset(n, c.center(s)).is_some())
                .collect();
            slot.insert(mine.iter().map(|c| c.center(s)).collect());
        }
    }
    // Every candidate of a pair shares that pair's footprint — including its narrow ones,
    // since the ladder may settle on any of them.
    for neck in necks.iter_mut() {
        if let Some(pts) = spans.get(&neck.pair) {
            neck.claimed = pts.clone();
        }
    }
    // The outline must lock these on their own hex corners, not project them onto a
    // room's wall: they lie outside that room's geometry, and projecting undoes the fill.
    // `spans` is keyed by exactly the pairs with a corridor candidate (it is filled from
    // `necks.iter().filter(is_corridor)` above), so it doubles as the set `fused_necks`
    // stands aside for — no second pass, and the two cannot drift apart.
    let mut join_cells = fused_necks(areas, &spans, s);
    join_cells.extend(areas.join().iter().copied());
    (Fusion { necks, cell: s }, join_cells)
}

impl Fusion {
    /// Whether a point sits on a NARROW connector — the only ones a door may be
    /// nudged aside for. A wide corridor spans a whole flank, and moving a door
    /// for one can fold the traced outline at the jamb however small the move, so
    /// a corridor lets the door stay where it is and breaks its own wall for the
    /// doorway instead — see [`Neck::jamb_edge`].
    pub(crate) fn blocks_narrow(&self, p: Point) -> bool {
        self.necks
            .iter()
            .filter(|n| !n.is_corridor())
            .any(|n| n.blocks(p))
    }

    /// Splice the accepted connectors into both layers that draw a wall, crop the
    /// class-D barrier residue, and commit growth's join floor — keeping only what a
    /// wall encloses, before water, stones and the floor pattern read the cell set.
    pub(crate) fn apply(
        self,
        outline: &mut [Vec<Point>],
        walls: &mut Vec<WallRun>,
        areas: &mut Areas,
        topology: &Topology,
    ) {
        let s = self.cell;
        let necks = splice_outline_necks(outline, self.necks);
        // Door cells (and the pillar a merged pair swallows) locate the doorway
        // gaps a connector's wall has to break for — see `Neck::jamb_edge`.
        let door_cells: Vec<Point> = topology
            .connections
            .iter()
            .map(|d| d.cell().center(s))
            .chain(topology.merged_doors.iter().map(|&(_, _, p)| p.center(s)))
            .collect();
        splice_necks(walls, &necks, &door_cells);
        // Links and junctions stitch as ONE pool: a link's chain continues across an adjacent
        // junction cell, so the two sections' walls join where they meet instead of each
        // stopping at its own boundary.
        let mut hex_segs = link_walls(areas, &topology.connections, s);
        hex_segs.extend(junction_walls(areas, s));
        walls.extend(stitch_hex_edges(hex_segs));
        // Clip every wall against the rooms it runs into. A corridor is built from the tiles the
        // two rooms share — room tiles — so its walls start out running inside both rooms, and a
        // fused pair's two borders overlap by construction. Neither stretch is wall: it is open
        // floor with a line across it. This replaces `crop_internal_barriers`, which did
        // the same thing for class-D pairs only, and by dropping interior vertices — leaving a
        // chord between the survivors — rather than by ending the run at the crossing.
        let rooms: Vec<ruins::RuinShape> = (0..areas.count())
            .filter_map(|i| areas.shape(i))
            .filter(|sh| {
                matches!(
                    sh,
                    ruins::RuinShape::Circle { .. } | ruins::RuinShape::Rect { .. }
                )
            })
            .collect();
        *walls = walls
            .iter()
            .flat_map(|run| crate::clip::split_outside(run, &rooms, 1.0))
            .collect();
        commit_join_floor(areas, walls, s);
    }
}
