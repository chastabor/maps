//! Ruins: geometric replacements for organic areas. A `ruins_level` fraction
//! of the (non-corridor) areas trade their cave-blob outline for a fitted
//! rectangle or circle. Boundary vertices of those areas are projected onto
//! the shape and locked against jitter, so walls come out straight for
//! rectangles and arcing for circles — including the passage mouths where
//! doors meet them.

use crate::AreaKind;
use crate::grid::{Hex, HexGrid};
use crate::growth::Areas;
use crate::outline::Point;
use crate::topology::Topology;
use rand::Rng;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RuinShape {
    Rect { cx: f64, cy: f64, hw: f64, hh: f64 },
    Circle { cx: f64, cy: f64, r: f64 },
    /// A corridor straightened into a hall: a thick segment from A to B.
    StraightHall { ax: f64, ay: f64, bx: f64, by: f64, hw: f64 },
    /// A corridor bent into a circular arc: an annulus band of half-width
    /// `hw` around radius `r`.
    ArcHall { cx: f64, cy: f64, r: f64, hw: f64 },
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
                let (abx, aby) = (bx - ax, by - ay);
                let len2 = (abx * abx + aby * aby).max(1e-9);
                let t = (((p.0 - ax) * abx + (p.1 - ay) * aby) / len2).clamp(0.0, 1.0);
                let q = (ax + abx * t, ay + aby * t);
                let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                let d = dx.hypot(dy).max(1e-9);
                (q.0 + dx / d * hw, q.1 + dy / d * hw)
            }
            RuinShape::ArcHall { cx, cy, r, hw } => {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                let d = dx.hypot(dy).max(1e-9);
                let rw = if d >= r { r + hw } else { r - hw };
                (cx + dx / d * rw, cy + dy / d * rw)
            }
        }
    }
}

impl RuinShape {
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
    /// face stays on the traced outline. Halls pass through unchanged.
    pub fn shrink(&self, d: f64) -> RuinShape {
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => RuinShape::Rect {
                cx,
                cy,
                hw: (hw - d).max(0.1),
                hh: (hh - d).max(0.1),
            },
            RuinShape::Circle { cx, cy, r } => RuinShape::Circle { cx, cy, r: (r - d).max(0.1) },
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
                let (dt, dr, db, dl) =
                    ((p.1 - y0).abs(), (x1 - p.0).abs(), (y1 - p.1).abs(), (p.0 - x0).abs());
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
            _ => Vec::new(),
        }
    }

    /// Arc-length distance from a perimeter point to the nearest corner
    /// (`INFINITY` for a shape with no corners, e.g. a circle).
    pub fn corner_dist(&self, p: Point) -> f64 {
        let Some(per) = self.perimeter() else { return f64::INFINITY };
        let t = self.wall_param(p);
        self.wall_corners()
            .into_iter()
            .map(|c| {
                let d = (t - c).rem_euclid(per);
                d.min(per - d)
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// Whether a pixel point is covered by the shape for rasterization,
    /// with a margin so the claimed cells extend slightly past the exact
    /// geometry — the traced cell boundary then lies outside the shape and
    /// projection only ever pulls walls inward, within the area's own cells.
    fn covers(&self, p: Point, s: f64) -> bool {
        let m = 0.35 * s;
        match *self {
            RuinShape::Rect { cx, cy, hw, hh } => {
                (p.0 - cx).abs() <= hw + m && (p.1 - cy).abs() <= hh + m
            }
            RuinShape::Circle { cx, cy, r } => (p.0 - cx).hypot(p.1 - cy) <= r + m,
            RuinShape::StraightHall { ax, ay, bx, by, hw: _ } => {
                let (abx, aby) = (bx - ax, by - ay);
                let len2 = (abx * abx + aby * aby).max(1e-9);
                let t = (((p.0 - ax) * abx + (p.1 - ay) * aby) / len2).clamp(0.0, 1.0);
                (p.0 - (ax + abx * t)).hypot(p.1 - (ay + aby * t)) <= 0.87 * s
            }
            RuinShape::ArcHall { cx, cy, r, hw: _ } => {
                ((p.0 - cx).hypot(p.1 - cy) - r).abs() <= 0.87 * s
            }
        }
    }
}

/// Area of a unit-side hexagon: 3*sqrt(3)/2.
const CELL_AREA: f64 = 2.598_076_211_353_316;

/// Fit each **ruin** area (see `Areas::kind`, classified before growth) with a
/// geometric shape (rooms: rectangles/circles; corridors: straight/arcing
/// halls) and *reshape its cells* to the rasterized shape, exactly as if the
/// node had grown that way. Shapes claim free cells, so a shape reaching
/// another area's neighbourhood unions with it at the cell level — the outline
/// then traces one loop around both and no interior border ever exists (but
/// never a free cell beside a **dungeon** room: its clean walls take no
/// seams). An area whose reshaping would disconnect it or orphan a door keeps
/// its organic cells and is **demoted back to organic**, so ruin ⇔ actually
/// reshaped always holds. Dungeon rooms grow as their final shape
/// (`growth::RoomPlan`) and are untouched here.
pub fn build<R: Rng>(
    areas: &mut Areas,
    topology: &Topology,
    grid: &HexGrid,
    hex_size: f64,
    rng: &mut R,
) {
    for i in 0..areas.count() {
        // Only ruins reshape. Dungeon rooms already grew as their final
        // geometry (growth::grow_room) and organic areas stay as grown.
        if areas.kind(i) != AreaKind::Ruin {
            continue;
        }
        let shape = if topology.is_corridor[i] {
            fit_hall(&areas.cells[i], hex_size, rng)
        } else {
            Some(fit(&areas.cells[i], hex_size, rng))
        };
        match shape {
            Some(shape) if reshape(areas, topology, grid, i, shape, hex_size) => {
                areas.set_shape(i, Some(shape));
            }
            _ => areas.set_kind(i, AreaKind::Organic),
        }
    }
}

/// Cell → shape map used for wall projection and decor classification.
/// Seam cells — where a reshaped area touches a *different* area — stay
/// organic: the merged throat then keeps its full cell width instead of
/// pinching down to the shapes' exact geometric intersection, and its walls
/// carry organic decoration. Cells the shape doesn't actually cover (the
/// door-adjacent originals kept for connectivity, which can sit entirely
/// outside the fitted geometry) stay organic too — projecting them would
/// collapse their connecting stub onto the distant shape wall.
pub fn ruin_cell_map(areas: &Areas, hex_size: f64) -> std::collections::HashMap<Hex, RuinShape> {
    let shapes = areas.shapes();
    let mut map = std::collections::HashMap::new();
    for (i, shape) in shapes.iter().enumerate() {
        let Some(shape) = shape else { continue };
        for &c in &areas.cells[i] {
            let seam = c
                .neighbors()
                .iter()
                .any(|n| areas.owner_of(*n).is_some_and(|o| o != i));
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

/// Rasterize `shape` onto the grid and make those cells area `i`'s cells.
/// Returns false (leaving the area untouched) if the result would be
/// disconnected or lose contact with one of the area's doors.
fn reshape(
    areas: &mut Areas,
    topology: &Topology,
    grid: &HexGrid,
    i: usize,
    shape: RuinShape,
    s: f64,
) -> bool {
    // Candidates: the original cells plus two rings around them — fitted
    // shapes are area-preserving, so they never reach further than that.
    let mut cand: HashSet<Hex> = areas.cells[i].iter().copied().collect();
    for _ in 0..2 {
        let ring: Vec<Hex> = cand
            .iter()
            .flat_map(|c| c.neighbors())
            .filter(|c| !cand.contains(c))
            .collect();
        cand.extend(ring);
    }

    let mut new_set: HashSet<Hex> = cand
        .into_iter()
        .filter(|&c| {
            grid.contains(c)
                && areas.owner_of(c).is_none_or(|o| o == i)
                && shape.covers(c.center(s), s)
                // A ruin may union with organic/ruin neighbours, but never
                // seam onto a dungeon room's clean wall: leave the rock gap.
                && c.neighbors()
                    .iter()
                    .all(|n| {
                        areas
                            .owner_of(*n)
                            .is_none_or(|o| o == i || areas.kind(o) != AreaKind::Dungeon)
                    })
        })
        .collect();

    // Door connectivity must survive: keep the original cells that touch
    // one of this area's doors or exit attachments.
    let anchors: Vec<Hex> = topology
        .doors
        .iter()
        .filter(|d| d.a == i || d.b == i)
        .map(|d| d.cell)
        .collect();
    for &c in &areas.cells[i] {
        let keep = topology.exits.iter().any(|e| e.area == i && e.attach == c)
            || anchors.iter().any(|a| a.neighbors().contains(&c));
        if keep {
            new_set.insert(c);
        }
    }

    if new_set.is_empty() {
        return false;
    }
    let mut cells: Vec<Hex> = new_set.iter().copied().collect();
    cells.sort_unstable();
    if !is_connected(&cells) {
        return false;
    }
    for a in &anchors {
        if !a.neighbors().iter().any(|nb| new_set.contains(nb)) {
            return false;
        }
    }
    // Reject shapes that enclose rock (a raster wrapped into a ring): the
    // pocket's walls and the region's outer walls would project onto the
    // same shape surfaces and pinch the floor between them shut.
    if encloses_rock(&new_set) {
        return false;
    }
    areas.replace_area(i, cells);
    true
}

/// True if the cell set surrounds any non-member cell: flood the non-member
/// cells of the set's (padded) bounding box from its border; anything
/// unreached is an enclosed pocket.
fn encloses_rock(cells: &HashSet<Hex>) -> bool {
    let (mut min_q, mut max_q, mut min_r, mut max_r) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for c in cells {
        min_q = min_q.min(c.q);
        max_q = max_q.max(c.q);
        min_r = min_r.min(c.r);
        max_r = max_r.max(c.r);
    }
    let (min_q, max_q, min_r, max_r) = (min_q - 1, max_q + 1, min_r - 1, max_r + 1);
    let in_box = |h: &Hex| h.q >= min_q && h.q <= max_q && h.r >= min_r && h.r <= max_r;

    let mut open: HashSet<Hex> = HashSet::new();
    let mut stack: Vec<Hex> = Vec::new();
    for q in min_q..=max_q {
        for r in min_r..=max_r {
            let h = Hex::new(q, r);
            let border = q == min_q || q == max_q || r == min_r || r == max_r;
            if border && !cells.contains(&h) && open.insert(h) {
                stack.push(h);
            }
        }
    }
    while let Some(c) = stack.pop() {
        for n in c.neighbors() {
            if in_box(&n) && !cells.contains(&n) && open.insert(n) {
                stack.push(n);
            }
        }
    }
    let box_cells = ((max_q - min_q + 1) * (max_r - min_r + 1)) as usize;
    box_cells - cells.len() != open.len()
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

/// Fit a shape over the area's cells, centred on the centroid and preserving
/// the cells' total area so the geometry stays inside the original footprint
/// and clear of neighbouring walls and doors.
fn fit<R: Rng>(cells: &[Hex], s: f64, rng: &mut R) -> RuinShape {
    let centers: Vec<Point> = cells.iter().map(|c| c.center(s)).collect();
    let n = centers.len() as f64;
    let cx = centers.iter().map(|c| c.0).sum::<f64>() / n;
    let cy = centers.iter().map(|c| c.1).sum::<f64>() / n;
    // Slightly under area-preserving: keeps the shape inside the original
    // footprint so neighbouring ruins merge deliberately, not constantly.
    let target = n * CELL_AREA * s * s * 0.8;

    if rng.random_bool(0.5) {
        let r = (target / std::f64::consts::PI).sqrt();
        RuinShape::Circle { cx, cy, r }
    } else {
        // Aspect ratio from the cell-centre bounding box, scaled to the
        // target area.
        let (mut min_x, mut min_y, mut max_x, mut max_y) =
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &centers {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let hw0 = ((max_x - min_x) / 2.0 + s * 0.87).max(s);
        let hh0 = ((max_y - min_y) / 2.0 + s * 0.87).max(s);
        let k = (target / (4.0 * hw0 * hh0)).sqrt();
        RuinShape::Rect {
            cx,
            cy,
            hw: (hw0 * k).max(s * 1.1),
            hh: (hh0 * k).max(s * 1.1),
        }
    }
}
