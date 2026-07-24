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
            RuinShape::Circle { cx, cy, r } => RuinShape::Circle { cx, cy, r: (r - d).max(0.1) },
            RuinShape::StraightHall { ax, ay, bx, by, hw } => {
                RuinShape::StraightHall { ax, ay, bx, by, hw: (hw - d).max(0.1) }
            }
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

/// Give each **ruin** area its final wall treatment. Ruin *rooms* already grew
/// from a flower into their exact rectangle/circle geometry — hex-aligned and
/// one rock cell from every neighbour, exactly like dungeon rooms — and
/// `growth::finalize` derived their [`RuinShape`]. Here we only **erode** them
/// (see [`erode`]): drop a fraction of boundary cells so the footprint weathers
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
            match fit_hall(&areas.cells[i], hex_size, rng) {
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
    let n0 = areas.cells[i].len();
    if n0 <= crate::growth::MIN_AREA {
        return;
    }
    // Cells that must survive so every door/exit still reaches this area.
    let mut anchors: HashSet<Hex> = HashSet::new();
    for e in &topology.exits {
        if e.area == i {
            anchors.insert(e.attach);
        }
    }
    for d in &topology.doors {
        if d.a == i || d.b == i {
            for &c in &areas.cells[i] {
                if c.neighbors().contains(&d.cell) {
                    anchors.insert(c);
                }
            }
        }
    }

    let target = ((n0 as f64) * EROSION_FRAC).round() as usize;
    let mut remaining: Vec<Hex> = areas.cells[i].clone();
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
        for &c in &areas.cells[i] {
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
