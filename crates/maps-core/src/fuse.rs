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
//! Along the way [`claim_corridor_floor`] gives a corridor floor to stand on and
//! [`release_unused_claims`] hands back what the accepted walls do not enclose.
//!
//! The design record, including the measurements behind the constants and the
//! approaches that were tried and rejected, is in `plans/fuse-case-taxonomy.md`.

use crate::geom::{self, Point};
use crate::grid::{self, HexGrid};
use crate::growth::Areas;
use crate::ruins;
use crate::topology::Topology;
use crate::AreaKind;

/// Cells forming the "neck" of every narrowly-fused dungeon pair: two dungeon
/// rooms that grew cell-adjacent but touch across only one or two faces. The
/// neck is the touching cells of both rooms. The outline locks these on their
/// raw hex corners (rather than projecting each side onto its own pinching room
/// wall), so the join is a full-hex-width, hex-aligned neck — the two touching
/// hexes are already floor, so nothing new is filled. Rooms touching across ≥3
/// faces already read as one compound and contribute nothing.
fn fused_necks(areas: &Areas) -> std::collections::HashSet<grid::Hex> {
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
        for &h in &areas.cells[a] {
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
    for (cells, faces) in seam.values() {
        if faces / 2 <= 2 {
            neck.extend(cells.iter().copied()); // narrow touch: give it a neck
        }
    }
    neck
}

/// Slop around a connector's footprint, in px: how far past its exact geometry
/// [`Neck::blocks`] still claims a vertex, and how far past the corridor span
/// [`claim_corridor_floor`] still admits a cell. The two must agree — the claimed
/// cells' vertices have to fall inside the footprint that replaces them, or floor
/// pokes past a wall.
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
    /// The floor cells [`claim_corridor_floor`] claimed for this pair, as
    /// `(cell centre, circumradius)` — empty until it runs.
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

    /// The hall's frame, shared by everything that has to agree on where the
    /// connector sits.
    fn frame(&self) -> Option<Frame> {
        let ruins::RuinShape::StraightHall { ax, ay, bx, by, .. } = self.hall else { return None };
        let d = (bx - ax, by - ay);
        let l = d.0.hypot(d.1).max(1e-9);
        Some(Frame { o: (ax, ay), len: l, dir: (d.0 / l, d.1 / l), nrm: (-d.1 / l, d.0 / l) })
    }

    /// Whether `p` lies on one of the wall lines this connector inserts — the
    /// stretch a door must not be cut into, since the splice overwrites it.
    ///
    /// Deliberately much tighter than [`blocks`](Self::blocks): a wide corridor's
    /// hall box spans the *whole* opening between the two rooms, but only the two
    /// new side walls are actually wall there.
    fn on_new_wall(&self, p: Point, tol: f64) -> bool {
        self.lines.iter().any(|&(a, b)| {
            let (_, q) = geom::project_on_segment(p, a, b);
            (p.0 - q.0).hypot(p.1 - q.1) <= tol
        })
    }

    /// Whether `p` falls inside the footprint this connector replaces — which
    /// outline/wall vertices the splice drops. The hall box, plus every hexagon
    /// whose floor the connector claimed (see [`Neck::claimed`]).
    fn blocks(&self, p: Point) -> bool {
        // A claimed cell's corners can reach past the hall box; its vertices are
        // the connector's to replace all the same.
        if self.claimed.iter().any(|&c| (p.0 - c.0).hypot(p.1 - c.1) <= self.cell + 0.5) {
            return true;
        }
        let ruins::RuinShape::StraightHall { hw, .. } = self.hall else { return false };
        let Some(f) = self.frame() else { return false };
        let (t, pp) = f.local(p);
        if pp.abs() > hw + FOOTPRINT_SLOP {
            return false;
        }
        if !self.is_corridor() {
            return t >= -FOOTPRINT_SLOP && t <= f.len + FOOTPRINT_SLOP;
        }
        // The two walls are generally UNEQUAL — each runs until it meets a border,
        // and a border curves — so the hall, a straight box between their midpoints,
        // is only as long as their *average* and does not contain them: the longer
        // wall's far end sticks out past it, outline vertices there escape the
        // splice, and the inserted wall then crosses them. Bound the span by
        // interpolating between the walls' own endpoints instead — the convex hull
        // of the two wall segments — so the footprint always holds both walls whole.
        // (The narrow angle neck keeps the hall box: its `lines` are not ordered
        // near-end-first, so this interpolation would not mean anything there.)
        let (n0, f0) = (f.local(self.lines[0].0), f.local(self.lines[0].1));
        let (n1, f1) = (f.local(self.lines[1].0), f.local(self.lines[1].1));
        let span = n1.1 - n0.1;
        let w = if span.abs() < 1e-9 { 0.5 } else { ((pp - n0.1) / span).clamp(0.0, 1.0) };
        let near = n0.0 + (n1.0 - n0.0) * w;
        let far = f0.0 + (f1.0 - f0.0) * w;
        t >= near.min(far) - FOOTPRINT_SLOP && t <= near.max(far) + FOOTPRINT_SLOP
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
        let d = (line.1 .0 - line.0 .0, line.1 .1 - line.0 .1);
        let len = d.0.hypot(d.1);
        if len < 1e-9 {
            return None;
        }
        let (u, nrm) = ((d.0 / len, d.1 / len), (-d.1 / len, d.0 / len));
        let par = |p: Point| (p.0 - line.0 .0) * u.0 + (p.1 - line.0 .1) * u.1;
        let perp = |p: Point| ((p.0 - line.0 .0) * nrm.0 + (p.1 - line.0 .1) * nrm.1).abs();
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
            .map(|e| (line.0 .0 + u.0 * e, line.0 .1 + u.1 * e))
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
    fn wall_stretch(&self, line: (Point, Point), prev: Point, next: Point) -> ((Point, bool), (Point, bool)) {
        let d = (line.1 .0 - line.0 .0, line.1 .1 - line.0 .1);
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
        ((at(s0), s0 == 0.0 || s0 == 1.0), (at(s1), s1 == 0.0 || s1 == 1.0))
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

/// Rows and vertical columns occupied by an area's cells. `interior_only` keeps
/// just the cells no wall cuts (all six neighbours in the same area).
///
/// Row key is `r` (constant `r` is constant `y`: a row of adjacent cells). The
/// column key is `2q + r`, NOT `q` — with `x = √3·s·(q + r/2)` a constant `q`
/// shifts x by `√3·s/2` per row, i.e. it is a DIAGONAL; constant `x` means
/// constant `2q + r`.
fn rows_cols(
    areas: &Areas,
    i: usize,
    interior_only: bool,
) -> (std::collections::HashSet<i32>, std::collections::HashSet<i32>) {
    let (mut rows, mut cols) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    for &h in &areas.cells[i] {
        if interior_only && !h.neighbors().iter().all(|nb| areas.owner_of(*nb) == Some(i)) {
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
/// Must be computed on the PRE-claim cells: `claim_corridor_floor` adds cells,
/// and feeding those back into `fuse_class` would let a corridor reclassify its
/// own pair.
fn fuse_pairs(areas: &Areas) -> Vec<(usize, usize, FuseClass)> {
    let mut touching: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    for a in 0..areas.count() {
        if areas.shape(a).is_none() {
            continue;
        }
        for &h in &areas.cells[a] {
            for nb in h.neighbors() {
                if let Some(b) =
                    areas.owner_of(nb).filter(|&b| b != a && areas.shape(b).is_some())
                {
                    touching.insert((a.min(b), a.max(b)));
                }
            }
        }
    }
    touching.into_iter().map(|(a, b)| (a, b, fuse_class(areas, a, b))).collect()
}

/// Whether two shapes' drawn borders overlap. A class-D pair that overlaps is
/// already open once its internal barrier is cropped and needs no connector; one
/// that does not still wants a corridor across the gap.
fn shapes_overlap(a: &ruins::RuinShape, b: &ruins::RuinShape) -> bool {
    use ruins::RuinShape as R;
    match (*a, *b) {
        (R::Circle { cx: x1, cy: y1, r: r1 }, R::Circle { cx: x2, cy: y2, r: r2 }) => {
            (x1 - x2).hypot(y1 - y2) < r1 + r2
        }
        (R::Rect { cx: x1, cy: y1, hw: w1, hh: h1 }, R::Rect { cx: x2, cy: y2, hw: w2, hh: h2 }) => {
            (x1 - x2).abs() < w1 + w2 && (y1 - y2).abs() < h1 + h2
        }
        (R::Circle { cx, cy, r }, R::Rect { cx: rx, cy: ry, hw, hh })
        | (R::Rect { cx: rx, cy: ry, hw, hh }, R::Circle { cx, cy, r }) => {
            let dx = ((cx - rx).abs() - hw).max(0.0);
            let dy = ((cy - ry).abs() - hh).max(0.0);
            dx.hypot(dy) < r
        }
        _ => false,
    }
}

/// How far along `n` a wall may sit and still cross this shape on a chord at least
/// two cells wide — `None` for anything but a circle, which is the only border that
/// can be grazed (a rect's sides meet a perpendicular wall squarely at every
/// offset).
///
/// The half-chord at offset `u` is `√(r² − (c·n − u)²)`, zero exactly at the
/// circle's extreme. Requiring a cell's span of it costs the corridor almost
/// nothing — the arc is nearly flat there, so for `r = 46` the bound moves in by
/// 1.6px — and it is what stops a wall leaving the border tangentially. Measured
/// over the sweep, halving the requirement leaves the worst wall-in-room case at
/// 13px instead of 10.5, so the full cell is the setting that earns its keep.
fn chord_bounds(sh: &ruins::RuinShape, n: Point, s: f64) -> Option<(f64, f64)> {
    let ruins::RuinShape::Circle { cx, cy, r } = *sh else { return None };
    let cn = cx * n.0 + cy * n.1;
    // Shortest half-chord worth a wall: one cell's span.
    let reach = (r * r - s * s).max(0.0).sqrt();
    Some((cn - reach, cn + reach))
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
        let (Some(sa), Some(sb)) = (areas.shape(a), areas.shape(b)) else { continue };
        // A class-D pair whose shapes already overlap is open once cropped.
        if class == FuseClass::Both && shapes_overlap(&sa, &sb) {
            continue;
        }
        // Circle↔rectangle corner fusions belong to `circle_rect_necks` (the
        // hex-aligned angle neck); routing them here too would double the wall.
        if class == FuseClass::Angle
            && matches!(
                (&sa, &sb),
                (ruins::RuinShape::Rect { .. }, ruins::RuinShape::Circle { .. })
                    | (ruins::RuinShape::Circle { .. }, ruins::RuinShape::Rect { .. })
            )
        {
            continue;
        }
        // Pick the axis PER PAIR: the one whose clamp is valid and widest. For
        // the diagonals this is the mirror rule from the FIX diagrams — a pair
        // offset the other way needs the mirrored axis, and choosing by clamp
        // width selects it automatically.
        let Some((&(d, n), clamp_lo, clamp_hi)) = axes
            .iter()
            .filter_map(|ax @ &(_, n)| {
                let (Some((a_lo, a_hi)), Some((b_lo, b_hi))) =
                    (support(&sa, n), support(&sb, n))
                else {
                    return None;
                };
                // A wall must cross a circle on a real CHORD, not graze it. The
                // clamp's inner bound is often the circle's own extreme along `n`,
                // and the wall there leaves the border at zero angle: the traced
                // outline wobbles across it, and the splice is refused for crossing
                // itself. `snap_wall` already moves the level corridor's bound off
                // the extreme as a side effect of landing on a lattice line, but a
                // flat normal is deliberately left unsnapped — measured, 400 clamp
                // bounds per sweep sat *exactly* tangent there.
                let (mut lo, mut hi) = (a_lo.max(b_lo), a_hi.min(b_hi));
                for sh in [&sa, &sb] {
                    if let Some((in_lo, in_hi)) = chord_bounds(sh, n, s) {
                        lo = lo.max(in_lo);
                        hi = hi.min(in_hi);
                    }
                }
                // Snap after, so the axis is chosen on the width the corridor will
                // really have (snapping only ever moves a bound further inward, so
                // it cannot undo the guard).
                let lo = snap_wall(lo, n, s, true);
                let hi = snap_wall(hi, n, s, false);
                (hi - lo > 0.0).then_some((ax, lo, hi))
            })
            .max_by(|x, y| (x.2 - x.1).total_cmp(&(y.2 - y.1)))
        else {
            continue;
        };
        // The nearer shape along `d` faces forward (+1) and the farther back.
        let (sl, sr) = if support(&sa, d) <= support(&sb, d) { (sa, sb) } else { (sb, sa) };
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
        for scale in (0..9).map(|i| 1.0 - 0.08 * i as f64) {
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
            let wall = |u: f64| -> Option<(Point, Point)> {
                Some((
                    at(u, border_along(&sl, n, u, d, 1.0)?),
                    at(u, border_along(&sr, n, u, d, -1.0)?),
                ))
            };
            let (Some(top), Some(bot)) = (wall(u_lo), wall(u_hi)) else { continue };
            // Hall: near-border midpoint → far-border midpoint, half-width the span.
            let mid = |x: Point, y: Point| ((x.0 + y.0) / 2.0, (x.1 + y.1) / 2.0);
            let (near, far) = (mid(top.0, bot.0), mid(top.1, bot.1));
            let hall = ruins::RuinShape::StraightHall {
                ax: near.0,
                ay: near.1,
                bx: far.0,
                by: far.1,
                hw: (u_hi - u_lo) / 2.0,
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
        let (Some(rect @ RuinShape::Rect { cx: rcx, cy: rcy, hw: rhw, hh: rhh }), Some(circ @ RuinShape::Circle { cx: ccx, cy: ccy, r: cr })) =
            (areas.shape(a), areas.shape(b))
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
            (disc >= 0.0).then(|| -bq - disc.sqrt()).filter(|&t| t > 0.0).map(|t| (p.0 + t * dir.0, p.1 + t * dir.1))
        };
        // The angle geometry below assumes the circle sits beyond a
        // left/right edge (x-dominant offset). A corner beyond a top/bottom
        // edge needs the transposed construction — deferred.
        if ((ccx - rcx) / rhw).abs() < ((ccy - rcy) / rhh).abs() {
            continue;
        }
        let Some(conn) = areas.cells[a]
            .iter()
            .filter(|h| h.neighbors().iter().any(|nb| areas.owner_of(*nb) == Some(b)))
            .copied()
            .min_by(|p, q| {
                (p.center(s).0 - near_x).abs().total_cmp(&(q.center(s).0 - near_x).abs())
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
        let (Some(c_hit), Some(h_hit)) = (hit(corner, dir), hit(hex_pt, dir)) else { continue };
        // A `StraightHall` whose two sides are the neck walls: centreline
        // midway between them, half-width the perpendicular half-distance.
        let mid0 = ((corner.0 + hex_pt.0) / 2.0, (corner.1 + hex_pt.1) / 2.0);
        let mid1 = ((c_hit.0 + h_hit.0) / 2.0, (c_hit.1 + h_hit.1) / 2.0);
        let nrm = (-dir.1, dir.0);
        let hw = ((corner.0 - hex_pt.0) * nrm.0 + (corner.1 - hex_pt.1) * nrm.1).abs() / 2.0;
        let hall = RuinShape::StraightHall { ax: mid0.0, ay: mid0.1, bx: mid1.0, by: mid1.1, hw };
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
            (self.lines[0].0 .0 + self.lines[0].1 .0) / 2.0,
            (self.lines[0].0 .1 + self.lines[0].1 .1) / 2.0,
        ))
        .signum();
        // A CLOSED sequence rotates to start on a kept vertex, else a dropped run
        // wrapping the seam would be split in two. An OPEN one must NOT rotate: it
        // is a polyline, not a cycle, so reordering it welds parts that are not
        // neighbours. (A band run is cut open at every doorway gap; rotating one
        // drew a wall straight across a room's interior, from the far end of its
        // arc back to the spliced stretch.) Its own ends are gap edges instead, so
        // `run_end` resolves both.
        let Some(first_kept) = (0..n).find(|&i| !self.blocks(pos(i))) else { return false };
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
fn splice_necks(
    walls: &mut [Vec<(Point, ruins::RuinShape)>],
    necks: &[Neck],
    doors: &[Point],
) {
    for neck in necks {
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
            if !run.iter().any(|v| v.1 == neck.shape_a || v.1 == neck.shape_b)
                || !run.iter().any(|v| neck.blocks(v.0))
            {
                continue;
            }
            let closed = run.len() > 2 && run.first().map(|v| v.0) == run.last().map(|v| v.0);
            let core = if closed { &run[..run.len() - 1] } else { &run[..] };
            let mut out: Vec<(Point, ruins::RuinShape)> = Vec::with_capacity(core.len() + 4);
            let ok = neck.splice_walk(
                core.len(),
                closed,
                &|i| core[i].0,
                // The band ran off the end of an open run, i.e. into a doorway
                // gap: ask the doorway where its jamb is, since the band's own
                // last vertex stops a cell short of it.
                &|line, last_dropped| {
                    neck.jamb_edge(line, doors, last_dropped).unwrap_or(last_dropped)
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
            if closed {
                if let Some(&first) = out.first() {
                    out.push(first);
                }
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
    let core = if closed { &loop_[..loop_.len() - 1] } else { &loop_[..] };
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
    if closed {
        if let Some(&first) = out.first() {
            out.push(first);
        }
    }
    Some((out, inserted))
}

/// Proper (transversal) intersection of segments `a→b` and `c→d`.
fn segs_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let o = |p: Point, q: Point, r: Point| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (s1, s2, s3, s4) =
        (o(a, b, c).signum(), o(a, b, d).signum(), o(c, d, a).signum(), o(c, d, b).signum());
    s1 != s2 && s3 != s4 && s1 != 0.0 && s2 != 0.0 && s3 != 0.0 && s4 != 0.0
}

/// Whether the walls a splice inserts cross anything else in the loop. Exact
/// rather than windowed: every segment touching an inserted vertex is tested
/// against every other segment, so a fold is caught however far apart the two
/// strands are in index order (a wide corridor's walls can reach right across a
/// loop). Cost is O(inserted · n), which is cheap because a splice inserts only
/// a handful of vertices.
fn inserted_walls_cross(pts: &[Point], inserted: &[usize]) -> bool {
    let m = pts.len();
    if m < 4 {
        return false;
    }
    let closed = pts[0] == pts[m - 1];
    let segs = m - 1;
    // Segments in the junction neighbourhood of an inserted vertex — not just the
    // two incident ones. Where an inserted wall endpoint lands almost on top of an
    // original vertex, the resulting spike is a crossing a couple of segments away
    // from the insertion itself.
    const NEAR: usize = 4;
    let mut probe: Vec<usize> = inserted
        .iter()
        .flat_map(|&i| i.saturating_sub(NEAR)..=i + NEAR)
        .filter(|&si| si < segs)
        .collect();
    probe.sort_unstable();
    probe.dedup();
    probe.iter().any(|&si| {
        (0..segs).any(|sj| {
            let apart = si.abs_diff(sj);
            // Skip self and index-adjacent pairs (they legitimately share an
            // endpoint), and the wrap-around pair of a closed loop.
            if apart <= 1 || (closed && apart == segs - 1) {
                return false;
            }
            segs_cross(pts[si], pts[si + 1], pts[sj], pts[sj + 1])
        })
    })
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
        let mut proposed: Vec<(usize, Vec<Point>)> = Vec::new();
        let mut safe = true;
        for (li, lp) in outline.iter().enumerate() {
            if let Some((new, inserted)) = spliced_loop(&neck, lp) {
                if inserted_walls_cross(&new, &inserted) {
                    safe = false;
                    break;
                }
                proposed.push((li, new));
            }
        }
        if safe && !proposed.is_empty() {
            for (li, new) in proposed {
                outline[li] = new;
            }
            placed.insert(neck.pair);
            kept.push(neck);
        }
    }
    kept
}

/// Claim the unowned floor a fused pair's corridor spans, and return the cells
/// claimed, per pair.
///
/// A connector redraws walls and the floor *outline* but never claims cells. On a
/// wide fusion that is invisible — the floor already spans the contact — and on a
/// narrow one it is the whole problem: a pair touching across a single hex face has
/// one hex of floor under a corridor several hexes long, so the corridor's walls run
/// through solid rock and cross the real floor boundary. The acceptance ladder then
/// degrades the corridor to a stub (the repro pair took the 0.36 rung, 25 units of a
/// 70-unit clamp). Filling the span first is what lets the full clamp stand.
///
/// The safety rule is [`growth::placeable`]'s one-cell rock gap relaxed exactly as
/// fusion relaxes it, and no further: a cell joins only if every neighbour is unowned
/// or owned by **this pair**. A cell abutting a third area would close that area's
/// rock gap with no door — the unintended-passage bug — and hand one of the pair a
/// second partner, breaking growth's fuse-once rule. Door, merged-pillar and
/// exit-stub cells stay unowned: they are floor already, and `topology` was fixed
/// before this runs, so it cannot react to losing one.
///
/// Filling proceeds a **line at a time, outward from the centreline**, stopping on
/// each side at the first line it cannot complete — see the walk below for why a half
/// line is worse than none. Only the *widest* candidate per pair claims (candidates
/// are widest-first), and only the wide corridors: the narrow hex-aligned angle
/// neck already runs along cells that are floor.
///
/// The claimed cells must be tagged with the **join's own hex boundary** rather
/// than either room's shape: a claimed cell lies outside both rooms' geometry, so
/// `splice_dungeon_runs` would project it back onto a room wall and silently undo
/// the fill. Returned per pair, which is also what [`release_unused_claims`] needs
/// afterwards.
fn claim_corridor_floor(
    areas: &mut Areas,
    grid: &HexGrid,
    topology: &Topology,
    necks: &mut [Neck],
    s: f64,
) -> std::collections::BTreeMap<(usize, usize), Vec<grid::Hex>> {
    use std::collections::{BTreeMap, HashSet};
    // Floor owned by the door/exit machinery rather than by an area.
    let mut reserved: HashSet<grid::Hex> = topology.doors.iter().map(|d| d.cell).collect();
    reserved.extend(topology.merged_doors.iter().map(|&(_, _, p)| p));
    for e in &topology.exits {
        reserved.extend(e.stub.iter().copied());
    }
    // Claims accumulate here first, so the neighbour rule sees earlier claims (a
    // cell one pair took is a third area to the next pair) before `areas` is
    // touched. `BTreeMap` keeps the applied order independent of hashing.
    let mut pending: BTreeMap<grid::Hex, usize> = BTreeMap::new();
    // Cells claimed per pair, so every candidate of that pair can be told about
    // them — the ladder may settle on a narrower rung than the one that claimed.
    let mut per_pair: BTreeMap<(usize, usize), Vec<grid::Hex>> = BTreeMap::new();
    for neck in necks.iter() {
        let (a, b) = neck.pair;
        if !neck.is_corridor() || per_pair.contains_key(&(a, b)) {
            continue;
        }
        let (Some(sa), Some(sb)) = (areas.shape(a), areas.shape(b)) else { continue };
        let (Some(f), ruins::RuinShape::StraightHall { hw, .. }) = (neck.frame(), neck.hall)
        else {
            continue;
        };
        // Every cell the corridor covers, grouped into the lattice **lines** that
        // run along it: cells sharing an offset across the corridor share a line,
        // whatever the axis, so one grouping serves level, upright and diagonal
        // alike (a level corridor's lines are hex rows; an upright corridor's are
        // the true vertical columns, whose cells sit two apart — see `rows_cols`).
        let mut lines: BTreeMap<i64, Vec<grid::Hex>> = BTreeMap::new();
        for &c in grid.cells() {
            let p = c.center(s);
            // Between the two rooms' borders along the axis — the ends are the
            // borders themselves, where the floor already is — and STRICTLY
            // between the two walls across it. A cell centred *on* a wall is half
            // outside it, and that is the common case rather than a fluke: the
            // clamp bounds are the rooms' own hex-aligned borders, which land on
            // cell centres. Claiming one wedges a hex-locked cell across the very
            // line the wall has to follow, which cost four seeds their connector
            // when measured.
            let (t, pp) = f.local(p);
            if t < -FOOTPRINT_SLOP || t > f.len + FOOTPRINT_SLOP || pp.abs() >= hw - 1e-6 {
                continue;
            }
            lines.entry((pp * 64.0).round() as i64).or_default().push(c);
        }
        // Work OUTWARD from the centreline, one line at a time, and stop each side
        // at the first line that cannot be completed. A line only counts if it ends
        // up floor of this pair all the way across — already theirs, claimable now,
        // or door/exit floor. Half a line is worse than none: it leaves a ragged
        // frontier mid-corridor for the outline to weave around, which is what the
        // walls then cross. Whole lines widen the passage evenly from the seam out.
        let keys: Vec<i64> = lines.keys().copied().collect();
        let split = keys.partition_point(|&k| k < 0);
        let outward = [keys[split..].to_vec(), keys[..split].iter().rev().copied().collect()];
        let mut got: Vec<grid::Hex> = Vec::new();
        for side in outward {
            for k in side {
                let mut take: Vec<grid::Hex> = Vec::new();
                let complete = lines[&k].iter().all(|&c| {
                    match areas.owner_of(c).or_else(|| pending.get(&c).copied()) {
                        Some(o) => o == a || o == b,
                        // Door, pillar and exit cells are floor already; leave them
                        // unowned (topology was fixed before this runs and cannot
                        // react to losing one) but let the line pass.
                        None if reserved.contains(&c) => true,
                        None => {
                            let owner_of = |h: grid::Hex| {
                                areas.owner_of(h).or_else(|| pending.get(&h).copied())
                            };
                            let safe = c
                                .neighbors()
                                .iter()
                                .all(|&nb| owner_of(nb).is_none_or(|o| o == a || o == b));
                            if safe {
                                take.push(c);
                            }
                            safe
                        }
                    }
                });
                if !complete {
                    break;
                }
                for c in take {
                    // Floor beside the circle belongs to the circle's room:
                    // whichever border it sits nearer takes it.
                    let p = c.center(s);
                    pending.insert(c, if sa.wall_dist(p) <= sb.wall_dist(p) { a } else { b });
                    got.push(c);
                }
            }
        }
        per_pair.insert((a, b), got);
    }
    for neck in necks.iter_mut() {
        if let Some(cells) = per_pair.get(&neck.pair) {
            neck.claimed = cells.iter().map(|c| c.center(s)).collect();
        }
    }
    let mut by_area: BTreeMap<usize, Vec<grid::Hex>> = BTreeMap::new();
    for (&c, &i) in &pending {
        by_area.entry(i).or_default().push(c);
    }
    for (i, add) in by_area {
        let mut cells = areas.cells[i].clone();
        cells.extend(add);
        areas.replace_area(i, cells);
    }
    per_pair.retain(|_, cells| !cells.is_empty());
    per_pair
}

/// Hand back the corridor floor the accepted connector turned out not to cover.
///
/// [`claim_corridor_floor`] claims for the **full** clamp, but the ladder may still
/// settle on a narrower rung, and then the outermost claimed cells lie beyond that
/// rung's walls — floor outside a wall (96 cells over the measured sweep, by up to
/// one hex). The floor *outline* is already right either way, since the connector's
/// footprint covers every cell it claimed whichever rung it ends on, so those
/// vertices are spliced away regardless. What is left is the cell set, which water,
/// stones and the floor pattern still read — so release the strays once the splice
/// has settled which corridor is real, and before any of those run.
///
/// Only the flanks are released. A claimed cell may also overhang the corridor's
/// *ends*, but those sit against the rooms' own borders, where floor belongs; and
/// dropping one would punch a hole in the corridor mouth.
///
/// A pair with no accepted connector keeps everything it claimed: there the claimed
/// cells *are* the join, carrying the hex-aligned neck the ladder was protecting.
fn release_unused_claims(
    areas: &mut Areas,
    necks: &[Neck],
    claimed: &std::collections::BTreeMap<(usize, usize), Vec<grid::Hex>>,
    s: f64,
) {
    for neck in necks {
        let Some(cells) = claimed.get(&neck.pair) else { continue };
        let (Some(f), ruins::RuinShape::StraightHall { hw, .. }) = (neck.frame(), neck.hall)
        else {
            continue;
        };
        for &c in cells {
            let p = c.center(s);
            let pp = f.local(p).1;
            if let Some(i) = areas.owner_of(c).filter(|_| pp.abs() >= hw) {
                areas.remove_from_area(i, &[c]);
            }
        }
    }
}

/// The shape pairs of every **class-D** fused pair — the ones
/// [`crop_internal_barriers`] may have a barrier to crop.
fn class_both_pairs(
    areas: &Areas,
    pairs: &[(usize, usize, FuseClass)],
) -> Vec<(ruins::RuinShape, ruins::RuinShape)> {
    pairs
        .iter()
        .filter(|&&(_, _, cls)| cls == FuseClass::Both)
        .filter_map(|&(a, b, _)| areas.shape(a).zip(areas.shape(b)))
        .collect()
}

/// Crop the internal barrier of a **class-D** fused pair: where two overlapping
/// shapes each project a wall through the other's interior, that stretch is a wall
/// standing in open floor. Drop those vertices so the compound reads as one space.
///
/// Interior vertices only — a run endpoint is never dropped, because every path is
/// expected to reach an area and removing an endpoint makes the renderer criss-cross
/// borders between passageways. Rare in practice (measured: 10 vertices over seeds
/// 1..=200), since the traced boundary already follows cell ownership; this catches
/// the residue where the *shapes* overlap even though the cells do not.
fn crop_internal_barriers(
    walls: &mut [Vec<(Point, ruins::RuinShape)>],
    pairs: &[(ruins::RuinShape, ruins::RuinShape)],
) {
    let inside = |sh: &ruins::RuinShape, p: Point| match *sh {
        ruins::RuinShape::Circle { cx, cy, r } => (p.0 - cx).hypot(p.1 - cy) < r - 1.0,
        ruins::RuinShape::Rect { cx, cy, hw, hh } => {
            (p.0 - cx).abs() < hw - 1.0 && (p.1 - cy).abs() < hh - 1.0
        }
        _ => false,
    };
    if pairs.is_empty() {
        return;
    }
    for run in walls.iter_mut() {
        if run.len() < 3 {
            continue;
        }
        let last = run.len() - 1;
        let mut i = 0;
        run.retain(|&(p, sh)| {
            let endpoint = i == 0 || i == last;
            i += 1;
            endpoint
                || !pairs.iter().any(|(sa, sb)| {
                    (sh == *sa && inside(sb, p)) || (sh == *sb && inside(sa, p))
                })
        });
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
    /// Class-D pairs, classified on the PRE-claim cells — the claim adds cells,
    /// and feeding those back into `fuse_class` would let a corridor reclassify
    /// its own pair.
    barriers: Vec<(ruins::RuinShape, ruins::RuinShape)>,
    claimed: std::collections::BTreeMap<(usize, usize), Vec<grid::Hex>>,
    cell: f64,
}

/// Classify every fused pair, build a connector for each, and claim the floor
/// those corridors need. Mutates `areas`: claimed cells join their room.
///
/// Returns the plan plus the cells the outline must lock on their own hex corners
/// — the narrow seams' and the claimed corridor floor's. Both belong to the join
/// rather than to either room, so projecting them onto a room wall would pinch the
/// seam shut or undo the fill.
pub(crate) fn plan(
    areas: &mut Areas,
    grid: &HexGrid,
    topology: &Topology,
    s: f64,
) -> (Fusion, std::collections::HashSet<grid::Hex>) {
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

    // Classify ONCE (pre-claim cells); every builder and the barrier list share it.
    let pairs = fuse_pairs(areas);
    let mut necks = axis_necks(areas, &pairs, FuseClass::Horiz, &LEVEL, s);
    necks.extend(axis_necks(areas, &pairs, FuseClass::Vert, &UPRIGHT, s));
    // Class D pairs whose shapes do NOT overlap: a diagonal corridor across the gap.
    necks.extend(axis_necks(areas, &pairs, FuseClass::Both, &DIAGONALS, s));
    necks.extend(axis_necks(areas, &pairs, FuseClass::Angle, &BOTH_AXES, s));
    necks.extend(circle_rect_necks(areas, &pairs, s));

    let mut join_cells = fused_necks(areas);
    let barriers = class_both_pairs(areas, &pairs);
    let claimed = claim_corridor_floor(areas, grid, topology, &mut necks, s);
    join_cells.extend(claimed.values().flatten().copied());
    (Fusion { necks, barriers, claimed, cell: s }, join_cells)
}

impl Fusion {
    /// Whether a point sits on a NARROW connector — the only ones a door may be
    /// nudged aside for. A wide corridor spans a whole flank, and moving a door
    /// for one can fold the traced outline at the jamb however small the move, so
    /// those yield to the door instead (dropped in [`apply`](Self::apply) if a
    /// mouth sits on one).
    pub(crate) fn blocks_narrow(&self, p: Point) -> bool {
        self.necks.iter().filter(|n| !n.is_corridor()).any(|n| n.blocks(p))
    }

    /// Splice the accepted connectors into both layers that draw a wall, crop the
    /// class-D barrier residue, and release claimed floor the accepted walls do
    /// not enclose (before water, stones and the floor pattern read the cell set).
    ///
    /// A wide connector first yields to any door already cut where its wall would
    /// go — the fusion falls back to the band merge for that pair rather than
    /// erasing a doorway.
    pub(crate) fn apply(
        self,
        outline: &mut [Vec<Point>],
        walls: &mut [Vec<(Point, ruins::RuinShape)>],
        areas: &mut Areas,
        topology: &Topology,
        mouths: &[crate::doorway::Mouth],
    ) {
        let s = self.cell;
        let kept: Vec<Neck> = self
            .necks
            .into_iter()
            .filter(|n| {
                !n.is_corridor()
                    || !mouths.iter().any(|mo| {
                        let h = mo.opening / 2.0;
                        [-1.0, 0.0, 1.0].iter().any(|f| {
                            n.on_new_wall(
                                (mo.center.0 + mo.axis.0 * h * f, mo.center.1 + mo.axis.1 * h * f),
                                s / 2.0,
                            )
                        })
                    })
            })
            .collect();
        let necks = splice_outline_necks(outline, kept);
        // Door cells (and the pillar a merged pair swallows) locate the doorway
        // gaps a connector's wall has to break for — see `Neck::jamb_edge`.
        let door_cells: Vec<Point> = topology
            .doors
            .iter()
            .map(|d| d.cell.center(s))
            .chain(topology.merged_doors.iter().map(|&(_, _, p)| p.center(s)))
            .collect();
        splice_necks(walls, &necks, &door_cells);
        crop_internal_barriers(walls, &self.barriers);
        release_unused_claims(areas, &necks, &self.claimed, s);
    }
}
