//! Ruins: geometric replacements for organic areas. A `ruins_level` fraction
//! of the (non-corridor) areas trade their cave-blob outline for a fitted
//! rectangle or circle. Boundary vertices of those areas are projected onto
//! the shape and locked against jitter, so walls come out straight for
//! rectangles and arcing for circles — including the passage mouths where
//! doors meet them.

use crate::AreaKind;
use crate::grid::Hex;
use crate::growth::Areas;
use crate::outline::Point;
use crate::topology::Topology;
use rand::Rng;
use rand::seq::SliceRandom;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RuinShape {
    Rect {
        cx: f64,
        cy: f64,
        hw: f64,
        hh: f64,
    },
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
    },
    /// A corridor straightened into a hall: a thick segment from A to B.
    StraightHall {
        ax: f64,
        ay: f64,
        bx: f64,
        by: f64,
        hw: f64,
    },
    /// A fusion corridor: two independent side walls, so a **trapezoid** rather than
    /// a box.
    ///
    /// Each wall runs from one room's border to the other's, and a curved border meets
    /// the two walls at different points along the corridor axis — so the walls are
    /// generally unequal in length and not parallel. A [`StraightHall`](Self::StraightHall)
    /// can only be as long as their average, which leaves the longer wall's end sticking
    /// out of its own footprint and the shorter one overshooting into a room.
    ///
    /// Both walls are stored **near-end first**: `.0` is the end on the room nearer along
    /// the corridor axis, `.1` the end on the farther room. Like a hall, the two caps are
    /// open mouths, not wall — the wall locus is the two side segments only, which is why
    /// [`perimeter`](Self::perimeter) is `None` here too.
    Trapezoid {
        wall0: (Point, Point),
        wall1: (Point, Point),
    },
    /// A corridor bent into a circular arc: an annulus band of half-width
    /// `hw` around radius `r`.
    ArcHall {
        cx: f64,
        cy: f64,
        r: f64,
        hw: f64,
    },
    /// One pointy-top hex cell's own boundary — circumradius `s`, corners at
    /// `60k − 30°`. Carried by the cells of a narrowly-fused seam so those
    /// vertices are *splicable* (they have a perimeter) while still sitting
    /// exactly where they already are: a neck vertex is on one of these corners
    /// already, so `project` is the identity for it. That keeps the raw-hex neck
    /// lock by construction instead of by opting out of the wall splice, which
    /// is what previously broke the band merge across a fused seam.
    HexCell {
        cx: f64,
        cy: f64,
        s: f64,
    },
}

/// Whether `p` is inside the quad the two walls bound, within `slop`.
///
/// The quad is `wall0.0 → wall0.1 → wall1.1 → wall1.0` — along one wall, across the far
/// cap, back along the other, across the near cap. It is convex (two non-crossing
/// segments joined end to end), so `p` is inside iff it is on the same side of all four
/// edges; the reference side is taken from the first edge rather than assumed, since
/// which wall is which side of the axis varies.
fn trapezoid_contains(wall0: (Point, Point), wall1: (Point, Point), p: Point, slop: f64) -> bool {
    let quad = [wall0.0, wall0.1, wall1.1, wall1.0];
    let mut sign = 0.0f64;
    for k in 0..4 {
        let (a, b) = (quad[k], quad[(k + 1) % 4]);
        let e = (b.0 - a.0, b.1 - a.1);
        let len = e.0.hypot(e.1);
        if len < 1e-9 {
            continue;
        }
        // Perpendicular distance, signed: scaling by `len` keeps `slop` in px.
        let side = ((p.0 - a.0) * e.1 - (p.1 - a.1) * e.0) / len;
        if sign == 0.0 && side.abs() > 1e-12 {
            sign = side.signum();
        }
        if sign != 0.0 && side * sign < -slop {
            return false;
        }
    }
    true
}

/// The narrowest perpendicular gap between two walls, measured at all four endpoints.
/// Non-parallel walls have no single width, and the minimum is what an inset must respect.
fn wall_separation(wall0: (Point, Point), wall1: (Point, Point)) -> f64 {
    let gap = |p: Point, w: (Point, Point)| {
        let (_, q) = crate::geom::project_on_segment(p, w.0, w.1);
        (p.0 - q.0).hypot(p.1 - q.1)
    };
    gap(wall0.0, wall1)
        .min(gap(wall0.1, wall1))
        .min(gap(wall1.0, wall0))
        .min(gap(wall1.1, wall0))
}

/// `wall` moved `d` toward `toward`, along `wall`'s own normal.
fn offset_wall(wall: (Point, Point), toward: (Point, Point), d: f64) -> (Point, Point) {
    let e = (wall.1.0 - wall.0.0, wall.1.1 - wall.0.1);
    let len = e.0.hypot(e.1).max(1e-9);
    let n = (-e.1 / len, e.0 / len);
    // Point the normal at the other wall's midpoint.
    let mid_other = (
        (toward.0.0 + toward.1.0) / 2.0,
        (toward.0.1 + toward.1.1) / 2.0,
    );
    let toward_other = (mid_other.0 - wall.0.0) * n.0 + (mid_other.1 - wall.0.1) * n.1;
    let sgn = if toward_other >= 0.0 { 1.0 } else { -1.0 };
    let (dx, dy) = (n.0 * d * sgn, n.1 * d * sgn);
    (
        (wall.0.0 + dx, wall.0.1 + dy),
        (wall.1.0 + dx, wall.1.1 + dy),
    )
}

/// The six corners of a pointy-top hex, in `wall_param` order (see
/// [`grid::hex_corner`](crate::grid::hex_corner) for the one corner convention).
fn hex_corners(cx: f64, cy: f64, s: f64) -> [Point; 6] {
    std::array::from_fn(|k| crate::grid::hex_corner((cx, cy), k, s))
}

/// Nearest point on a hex cell's boundary: `(edge index, offset within the
/// edge, the point)`. The one edge scan behind both `HexCell::project` and
/// `HexCell::wall_param`, so the two can never disagree about which edge a
/// point belongs to.
fn hex_edge_nearest(cx: f64, cy: f64, s: f64, p: Point) -> (usize, f64, Point) {
    let c = hex_corners(cx, cy, s);
    let mut best = (f64::MAX, 0, 0.0, p);
    for k in 0..6 {
        let (a, b) = (c[k], c[(k + 1) % 6]);
        let (t, q) = crate::geom::project_on_segment(p, a, b);
        let dist = (p.0 - q.0).hypot(p.1 - q.1);
        if dist < best.0 {
            best = (dist, k, t, q);
        }
    }
    (best.1, best.2, best.3)
}

impl RuinShape {
    /// Project a boundary point onto the shape's perimeter (rooms) or onto
    /// the nearest wall of the hall (corridors).
    pub fn project(&self, p: Point) -> Point {
        match *self {
            RuinShape::Circle { cx, cy, r } => {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                let d = dx.hypot(dy).max(1e-9);
                (cx + dx / d * r, cy + dy / d * r)
            }
            RuinShape::Rect { cx, cy, hw, hh } => {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                if dx.abs() > hw || dy.abs() > hh {
                    // Exterior: nearest point on the perimeter. A point
                    // beyond a corner lands on the *exact* corner, so walls
                    // meet at a sharp 90° (a radial map would spread the
                    // corner quadrant over both edges — a chamfer).
                    (cx + dx.clamp(-hw, hw), cy + dy.clamp(-hh, hh))
                } else {
                    // Interior (the trace cuts inside where a corner cell is
                    // unfilled): push outward along the centre ray, which
                    // lands the diagonal cut near the corner. Nearest-edge
                    // would split the cut across both edges and re-chamfer.
                    let k = (dx.abs() / hw).max(dy.abs() / hh).max(1e-9);
                    (cx + dx / k, cy + dy / k)
                }
            }
            RuinShape::StraightHall { ax, ay, bx, by, hw } => {
                let (_, q) = crate::geom::project_on_segment(p, (ax, ay), (bx, by));
                let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                let d = dx.hypot(dy).max(1e-9);
                (q.0 + dx / d * hw, q.1 + dy / d * hw)
            }
            // The wall locus is the two side segments, so project onto the nearer one.
            RuinShape::Trapezoid { wall0, wall1 } => {
                let (_, q0) = crate::geom::project_on_segment(p, wall0.0, wall0.1);
                let (_, q1) = crate::geom::project_on_segment(p, wall1.0, wall1.1);
                let (d0, d1) = (
                    (p.0 - q0.0).hypot(p.1 - q0.1),
                    (p.0 - q1.0).hypot(p.1 - q1.1),
                );
                if d0 <= d1 { q0 } else { q1 }
            }
            RuinShape::ArcHall { cx, cy, r, hw } => {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                let d = dx.hypot(dy).max(1e-9);
                let rw = if d >= r { r + hw } else { r - hw };
                (cx + dx / d * rw, cy + dy / d * rw)
            }
            // Nearest point on the six edges. A neck vertex already sits on a
            // corner, so this returns it unchanged — the raw-hex lock.
            RuinShape::HexCell { cx, cy, s } => hex_edge_nearest(cx, cy, s, p).2,
        }
    }
}

impl RuinShape {
    /// Whether `p` lies within the space this shape walls in — inside the perimeter
    /// for a room, between the side walls for a hall, inside the hexagon for a cell.
    ///
    /// Exact. Distinct from its two neighbours: [`wall_dist`](Self::wall_dist) is
    /// unsigned and so cannot tell inside from out, and `covers` is a rasterization test
    /// that deliberately admits a margin and ignores a hall's real half-width. Used to
    /// answer "is this floor enclosed by anything?".
    pub fn contains(&self, p: Point) -> bool {
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => {
                (p.0 - cx).abs() <= hw + 1e-6 && (p.1 - cy).abs() <= hh + 1e-6
            }
            RuinShape::Circle { cx, cy, r } => (p.0 - cx).hypot(p.1 - cy) <= r + 1e-6,
            RuinShape::StraightHall { ax, ay, bx, by, hw } => {
                let (_, q) = crate::geom::project_on_segment(p, (ax, ay), (bx, by));
                (p.0 - q.0).hypot(p.1 - q.1) <= hw + 1e-6
            }
            RuinShape::Trapezoid { wall0, wall1 } => trapezoid_contains(wall0, wall1, p, 1e-6),
            RuinShape::ArcHall { cx, cy, r, hw } => {
                ((p.0 - cx).hypot(p.1 - cy) - r).abs() <= hw + 1e-6
            }
            // A pointy-top hex contains `p` iff `p` is within the apothem of the
            // centre on all three edge normals (0° and ±60°).
            RuinShape::HexCell { cx, cy, s } => {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                let a = crate::grid::HEX_APOTHEM * s + 1e-6;
                [
                    (1.0, 0.0),
                    (0.5, crate::grid::HEX_APOTHEM),
                    (-0.5, crate::grid::HEX_APOTHEM),
                ]
                .iter()
                .all(|&(nx, ny)| (dx * nx + dy * ny).abs() <= a)
            }
        }
    }

    /// Distance from a pixel point to the shape's wall locus (the perimeter
    /// for rooms, the two side walls for halls). Used to classify wall decor
    /// samples geometrically — a cell lookup misses e.g. a rectangle's
    /// corners, which no hex cell contains.
    pub fn wall_dist(&self, p: Point) -> f64 {
        match *self {
            // Nearest-edge inside, clamped-perimeter outside — distinct from
            // `project`'s radial interior push, so it can't defer to it.
            RuinShape::Rect { cx, cy, hw, hh } => {
                let (ox, oy) = ((p.0 - cx).abs() - hw, (p.1 - cy).abs() - hh);
                if ox > 0.0 || oy > 0.0 {
                    ox.max(0.0).hypot(oy.max(0.0))
                } else {
                    (-ox).min(-oy)
                }
            }
            // The other walls are equidistant loci, so the distance is just
            // how far the point moved when projected onto them.
            _ => {
                let q = self.project(p);
                (p.0 - q.0).hypot(p.1 - q.1)
            }
        }
    }

    /// The room shape offset inward by `d`: the locus of points `d` inside
    /// the wall. Strokes of width `2d` centred on it span exactly from the
    /// wall to `2d` inside — the inward-thick dungeon wall band, whose outer
    /// face stays on the traced outline. A `StraightHall` narrows its
    /// half-width (its two side walls move `d` toward the centreline) — used
    /// by the circle↔rectangle fusion connector to get its inner wall line;
    /// `ArcHall` passes through unchanged.
    pub fn shrink(&self, d: f64) -> RuinShape {
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => RuinShape::Rect {
                cx,
                cy,
                hw: (hw - d).max(0.1),
                hh: (hh - d).max(0.1),
            },
            RuinShape::Circle { cx, cy, r } => RuinShape::Circle {
                cx,
                cy,
                r: (r - d).max(0.1),
            },
            RuinShape::StraightHall { ax, ay, bx, by, hw } => RuinShape::StraightHall {
                ax,
                ay,
                bx,
                by,
                hw: (hw - d).max(0.1),
            },
            // Each wall moves `d` toward the other along its OWN normal, so the inner
            // faces stay parallel to their own wall rather than to a shared centreline.
            // Non-parallel walls therefore give non-parallel inner faces, which is what
            // keeps the band flush on both sides of a wedge-shaped corridor.
            //
            // Clamped so the two never cross: the band is drawn 0.6 of a cell thick while
            // the narrowest accepted corridor is one cell wide, so an unclamped inset
            // would turn the trapezoid inside out. `StraightHall` leans on its
            // `max(0.1)` for the same reason.
            RuinShape::Trapezoid { wall0, wall1 } => {
                let room = wall_separation(wall0, wall1) / 2.0 - 0.1;
                let d = d.min(room.max(0.0));
                RuinShape::Trapezoid {
                    wall0: offset_wall(wall0, wall1, d),
                    wall1: offset_wall(wall1, wall0, d),
                }
            }
            // Inset: the apothem (√3/2·s) drops by `d`, so s' = s − 2d/√3.
            RuinShape::HexCell { cx, cy, s } => RuinShape::HexCell {
                cx,
                cy,
                s: (s - 2.0 * d / crate::grid::SQRT3).max(0.1),
            },
            other => other,
        }
    }

    /// Wall length of a **room** shape (rect perimeter / circle circumference);
    /// `None` for halls, which have no closed room wall. The room shapes are
    /// exactly those whose walls the outline splices onto exact geometry, so
    /// `perimeter().is_some()` is the single source of truth for "splicable".
    pub fn perimeter(&self) -> Option<f64> {
        match *self {
            RuinShape::Rect { hw, hh, .. } => Some(4.0 * (hw + hh)),
            RuinShape::Circle { r, .. } => Some(std::f64::consts::TAU * r),
            RuinShape::HexCell { s, .. } => Some(6.0 * s),
            _ => None,
        }
    }

    /// Arc-length parameter of a perimeter point (pass a point through
    /// `project` first). Rect walls run top L→R, right T→B, bottom R→L, left
    /// B→T with corners at the seams; circles run by angle from +x. Meaningful
    /// only where `perimeter()` is `Some`.
    pub fn wall_param(&self, p: Point) -> f64 {
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => {
                let (x0, x1, y0, y1) = (cx - hw, cx + hw, cy - hh, cy + hh);
                let (w, h) = (2.0 * hw, 2.0 * hh);
                let (dt, dr, db, dl) = (
                    (p.1 - y0).abs(),
                    (x1 - p.0).abs(),
                    (y1 - p.1).abs(),
                    (p.0 - x0).abs(),
                );
                let m = dt.min(dr).min(db).min(dl);
                if m == dt {
                    (p.0 - x0).clamp(0.0, w)
                } else if m == dr {
                    w + (p.1 - y0).clamp(0.0, h)
                } else if m == db {
                    w + h + (x1 - p.0).clamp(0.0, w)
                } else {
                    2.0 * w + h + (y1 - p.1).clamp(0.0, h)
                }
            }
            RuinShape::Circle { cx, cy, r } => {
                (p.1 - cy).atan2(p.0 - cx).rem_euclid(std::f64::consts::TAU) * r
            }
            // Edge `k` spans [k·s, (k+1)·s); every edge of a regular hexagon is
            // exactly `s` long, so the offset within an edge is its own length.
            RuinShape::HexCell { cx, cy, s } => {
                let (k, t, _) = hex_edge_nearest(cx, cy, s, p);
                (k as f64 + t) * s
            }
            _ => 0.0,
        }
    }

    /// Inverse of [`wall_param`](Self::wall_param).
    pub fn wall_point(&self, t: f64) -> Point {
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => {
                let (x0, x1, y0, y1) = (cx - hw, cx + hw, cy - hh, cy + hh);
                let (w, h) = (2.0 * hw, 2.0 * hh);
                let t = t.rem_euclid(4.0 * (hw + hh));
                if t < w {
                    (x0 + t, y0)
                } else if t < w + h {
                    (x1, y0 + (t - w))
                } else if t < 2.0 * w + h {
                    (x1 - (t - w - h), y1)
                } else {
                    (x0, y1 - (t - 2.0 * w - h))
                }
            }
            RuinShape::Circle { cx, cy, r } => {
                let a = t / r;
                (cx + r * a.cos(), cy + r * a.sin())
            }
            RuinShape::HexCell { cx, cy, s } => {
                let c = hex_corners(cx, cy, s);
                let u = (t / s.max(1e-9)).rem_euclid(6.0);
                let (k, f) = (u.floor() as usize % 6, u.fract());
                let (a, b) = (c[k], c[(k + 1) % 6]);
                (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
            }
            _ => (0.0, 0.0),
        }
    }

    /// Arc-length positions of a rect's corners (the wall-parameter seams);
    /// empty for shapes without corners. Feature points for wall resampling.
    pub fn wall_corners(&self) -> Vec<f64> {
        match *self {
            RuinShape::Rect { hw, hh, .. } => {
                let (w, h) = (2.0 * hw, 2.0 * hh);
                vec![0.0, w, w + h, 2.0 * w + h]
            }
            RuinShape::HexCell { s, .. } => (0..6).map(|k| k as f64 * s).collect(),
            _ => Vec::new(),
        }
    }

    /// Whether a pixel point is covered by the shape for rasterization,
    /// with a margin so the claimed cells extend slightly past the exact
    /// geometry — the traced cell boundary then lies outside the shape and
    /// projection only ever pulls walls inward, within the area's own cells.
    ///
    /// **Cell-granularity, not exact** — and deliberately so, which is the difference
    /// from [`contains`](Self::contains): rooms get a `0.35·s` margin, and both hall
    /// variants substitute a cell-scale band for the hall's actual half-width. Use
    /// `contains` for an exact test.
    fn covers(&self, p: Point, s: f64) -> bool {
        let m = 0.35 * s;
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => {
                (p.0 - cx).abs() <= hw + m && (p.1 - cy).abs() <= hh + m
            }
            RuinShape::Circle { cx, cy, r } => (p.0 - cx).hypot(p.1 - cy) <= r + m,
            RuinShape::StraightHall {
                ax,
                ay,
                bx,
                by,
                hw: _,
            } => {
                let (_, q) = crate::geom::project_on_segment(p, (ax, ay), (bx, by));
                (p.0 - q.0).hypot(p.1 - q.1) <= 0.87 * s
            }
            RuinShape::Trapezoid { wall0, wall1 } => trapezoid_contains(wall0, wall1, p, 0.87 * s),
            RuinShape::ArcHall { cx, cy, r, hw: _ } => {
                ((p.0 - cx).hypot(p.1 - cy) - r).abs() <= 0.87 * s
            }
            // Inside the hexagon: within the apothem on all three axes.
            RuinShape::HexCell { cx, cy, s: hs } => {
                let (dx, dy) = ((p.0 - cx).abs(), (p.1 - cy).abs());
                let ap = crate::grid::SQRT3 / 2.0 * hs + m;
                dy <= hs + m
                    && dx <= ap
                    && crate::grid::SQRT3 * ap - crate::grid::SQRT3 * dx >= dy - hs - m
            }
        }
    }
}

/// Give each **ruin** area its final wall treatment. Ruin *rooms* already grew
/// from a flower into their exact rectangle/circle geometry — hex-aligned and
/// one rock cell from every neighbour, exactly like dungeon rooms — and
/// `growth::finalize` derived their [`RuinShape`]. Here we only **erode** them
/// (see `erode`): drop a fraction of boundary cells so the footprint weathers
/// into organic bites while the intact walls still project onto the clean shape
/// (the soft ruin projection in [`ruin_cell_map`], vs a dungeon's hard-locked
/// wall). A ruin that `topology` shrank into a corridor is refitted as a
/// straight/arcing hall instead; if no hall fits it is demoted back to organic.
/// Dungeon and organic areas are untouched here.
pub fn build<R: Rng>(areas: &mut Areas, topology: &Topology, hex_size: f64, rng: &mut R) {
    for i in 0..areas.count() {
        if areas.kind(i) != AreaKind::Ruin {
            continue;
        }
        if topology.is_corridor[i] {
            // A shrunk ruin's grown rect/circle no longer describes its cells:
            // refit a hall, or demote to organic (dropping the stale shape).
            // The ROOM's cells: `fit_hall` runs the hall through the farthest pair of
            // cell centres, so a corridor cell reaching toward a partner would stretch it
            // past the cells it is meant to describe. (No measured map does this today —
            // a shrunk ruin that is also fused has not come up over 200 seeds — so this
            // states the intent rather than fixing an observed defect.)
            let room: Vec<Hex> = areas.room_cells(i).collect();
            match fit_hall(&room, hex_size, rng) {
                Some(hall) => areas.set_shape(i, Some(hall)),
                None => {
                    areas.set_kind(i, AreaKind::Organic);
                    areas.set_shape(i, None);
                }
            }
        } else {
            // Room ruin: keep its finalize-derived clean shape, erode the walls.
            erode(areas, topology, i, rng);
        }
    }
}

/// Fraction of a ruin room's cells to nibble off its boundary — enough to read
/// as weathered without dissolving the rectangle/circle.
const EROSION_FRAC: f64 = 0.18;

/// Weather a ruin room: remove up to `EROSION_FRAC` of its boundary cells,
/// leaving organic bites in the otherwise-clean wall. Never removes a cell that
/// would disconnect the area, drop it below [`crate::growth::MIN_AREA`], or is
/// needed to keep one of its doors/exits reachable. Freed cells become rock, so
/// the footprint only shrinks — erosion can never re-introduce an overlap with
/// a neighbour. The derived [`RuinShape`] is left in place: intact walls still
/// project onto it while the bites read organic.
fn erode<R: Rng>(areas: &mut Areas, topology: &Topology, i: usize, rng: &mut R) {
    let n0 = areas.floor_cells(i).count();
    if n0 <= crate::growth::MIN_AREA {
        return;
    }
    // Cells that must survive so every door/exit still reaches this area.
    let mut anchors: HashSet<Hex> = HashSet::new();
    // Fusion-corridor floor is one of them. It is not this room's wall — it is the join
    // to a partner, and it is always a boundary cell (that is what a protrusion is), so
    // erosion would pick it first and dissolve the fusion the corridor exists to carry.
    // This is how a fused pair used to lose a side; `fuse::Fusion::release_orphans` is
    // the backstop for what this now prevents.
    anchors.extend(areas.floor_cells(i).filter(|&c| areas.is_join(c)));
    for e in &topology.exits {
        if e.area == i {
            anchors.insert(e.attach);
        }
    }
    for d in &topology.doors {
        if d.a == i || d.b == i {
            for c in areas.floor_cells(i) {
                if c.neighbors().contains(&d.cell) {
                    anchors.insert(c);
                }
            }
        }
    }

    let target = ((n0 as f64) * EROSION_FRAC).round() as usize;
    let mut remaining: Vec<Hex> = areas.floor_cells(i).collect();
    let mut removed = 0;
    while removed < target && remaining.len() > crate::growth::MIN_AREA {
        let set: HashSet<Hex> = remaining.iter().copied().collect();
        // Boundary, non-anchor candidates: a cell with a non-member neighbour.
        let mut cand: Vec<Hex> = remaining
            .iter()
            .copied()
            .filter(|c| !anchors.contains(c) && c.neighbors().iter().any(|n| !set.contains(n)))
            .collect();
        cand.sort_unstable(); // canonical order before the seeded shuffle
        cand.shuffle(rng);
        let mut progressed = false;
        for cell in cand {
            let test: Vec<Hex> = remaining.iter().copied().filter(|&x| x != cell).collect();
            if is_connected(&test) {
                remaining = test;
                removed += 1;
                progressed = true;
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    if removed > 0 {
        remaining.sort_unstable();
        areas.replace_area(i, remaining);
    }
}

/// Cell → shape map used for wall projection and decor classification.
/// Seam cells — where a reshaped area touches a *different* area — stay
/// organic: the merged throat then keeps its full cell width instead of
/// pinching down to the shapes' exact geometric intersection, and its walls
/// carry organic decoration. Cells the shape doesn't actually cover (the
/// door-adjacent originals kept for connectivity, which can sit entirely
/// outside the fitted geometry) stay organic too — projecting them would
/// collapse their connecting stub onto the distant shape wall. Cells bordering
/// an **erosion bite** (a freed rock cell *inside* the shape) also stay organic:
/// the bite's weathered rim must read as a crumbled break, and hard-projecting
/// its rim onto the intact wall line would fold the loop over the bite.
pub fn ruin_cell_map(areas: &Areas, hex_size: f64) -> std::collections::HashMap<Hex, RuinShape> {
    let shapes = areas.shapes();
    let mut map = std::collections::HashMap::new();
    for (i, shape) in shapes.iter().enumerate() {
        let Some(shape) = shape else { continue };
        for c in areas.floor_cells(i) {
            let seam = c.neighbors().iter().any(|n| match areas.owner_of(*n) {
                // Borders a different area.
                Some(o) => o != i,
                // Borders an erosion bite: free rock that lies inside the shape.
                None => shape.covers(n.center(hex_size), hex_size),
            });
            // A cell inside a *second* ruin's geometry sits in the broken
            // zone where two structures ran into each other: projecting it
            // would extend this shape's wall across the other shape's wall
            // locus and tie the boundary into a bowtie.
            let contested = shapes.iter().enumerate().any(|(j, s)| {
                j != i && s.is_some_and(|s2| s2.covers(c.center(hex_size), hex_size))
            });
            if !seam && !contested && shape.covers(c.center(hex_size), hex_size) {
                map.insert(c, *shape);
            }
        }
    }
    map
}

fn is_connected(cells: &[Hex]) -> bool {
    if cells.is_empty() {
        return true;
    }
    let set: HashSet<Hex> = cells.iter().copied().collect();
    let mut seen: HashSet<Hex> = HashSet::from([cells[0]]);
    let mut stack = vec![cells[0]];
    while let Some(c) = stack.pop() {
        for m in c.neighbors() {
            if set.contains(&m) && seen.insert(m) {
                stack.push(m);
            }
        }
    }
    seen.len() == cells.len()
}

/// Fit a corridor with a hall: straight (thick segment between its two
/// farthest cells) or arching (annulus through them, bulging the way the
/// corridor already bulges). Returns `None` when the corridor deviates too
/// far from the fitted centreline — forcing it would drag walls across
/// neighbouring geometry.
fn fit_hall<R: Rng>(cells: &[Hex], s: f64, rng: &mut R) -> Option<RuinShape> {
    let centers: Vec<Point> = cells.iter().map(|c| c.center(s)).collect();
    if centers.len() < 3 {
        return None;
    }
    // Farthest pair of cell centres = the hall's endpoints.
    let (mut a, mut b, mut best) = (centers[0], centers[0], -1.0);
    for i in 0..centers.len() {
        for j in i + 1..centers.len() {
            let d = (centers[i].0 - centers[j].0).hypot(centers[i].1 - centers[j].1);
            if d > best {
                best = d;
                a = centers[i];
                b = centers[j];
            }
        }
    }
    let hw = 0.55 * s;
    let max_dev = 1.6 * s;

    let perp = |p: &Point| {
        let (abx, aby) = (b.0 - a.0, b.1 - a.1);
        let len = abx.hypot(aby).max(1e-9);
        ((p.0 - a.0) * aby - (p.1 - a.1) * abx) / len
    };

    if rng.random_bool(0.5) {
        // Arc through the endpoints and the corridor's most-bulged cell.
        let apex = centers
            .iter()
            .cloned()
            .max_by(|p, q| perp(p).abs().total_cmp(&perp(q).abs()))
            .unwrap();
        // A workable arc needs a radius of several cells: any smaller and
        // the rasterized band wraps into a full ring around the centre,
        // whose enclosed pocket pinches shut under projection.
        if perp(&apex).abs() > 0.8 * s
            && let Some((center, r)) = circumcircle(a, apex, b)
            && r < best * 4.0
            && r >= 2.5 * s
        {
            let fits = centers
                .iter()
                .all(|p| ((p.0 - center.0).hypot(p.1 - center.1) - r).abs() <= max_dev);
            if fits {
                return Some(RuinShape::ArcHall {
                    cx: center.0,
                    cy: center.1,
                    r,
                    hw,
                });
            }
        }
    }

    // Straight hall, endpoints pushed out to cover the end cells' walls.
    if centers.iter().all(|p| perp(p).abs() <= max_dev) {
        let (abx, aby) = (b.0 - a.0, b.1 - a.1);
        let len = abx.hypot(aby).max(1e-9);
        let (ux, uy) = (abx / len, aby / len);
        let pad = 0.6 * s;
        Some(RuinShape::StraightHall {
            ax: a.0 - ux * pad,
            ay: a.1 - uy * pad,
            bx: b.0 + ux * pad,
            by: b.1 + uy * pad,
            hw,
        })
    } else {
        None
    }
}

/// Circumcircle of three points; `None` when they are nearly collinear.
fn circumcircle(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    let d = 2.0 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1));
    if d.abs() < 1e-6 {
        return None;
    }
    let a2 = a.0 * a.0 + a.1 * a.1;
    let b2 = b.0 * b.0 + b.1 * b.1;
    let c2 = c.0 * c.0 + c.1 * c.1;
    let ux = (a2 * (b.1 - c.1) + b2 * (c.1 - a.1) + c2 * (a.1 - b.1)) / d;
    let uy = (a2 * (c.0 - b.0) + b2 * (a.0 - c.0) + c2 * (b.0 - a.0)) / d;
    let r = (a.0 - ux).hypot(a.1 - uy);
    Some(((ux, uy), r))
}
