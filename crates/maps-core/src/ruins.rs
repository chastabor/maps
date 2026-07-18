//! Ruins: geometric replacements for organic areas. A `ruins_level` fraction
//! of the (non-corridor) areas trade their cave-blob outline for a fitted
//! rectangle or circle. Boundary vertices of those areas are projected onto
//! the shape and locked against jitter, so walls come out straight for
//! rectangles and arcing for circles — including the passage mouths where
//! doors meet them.

use crate::grid::{Hex, HexGrid};
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
                let k = (dx.abs() / hw).max(dy.abs() / hh).max(1e-9);
                (cx + dx / k, cy + dy / k)
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

/// Pick a `level` fraction of the areas, fit each with a geometric shape
/// (rooms: rectangles/circles; corridors: straight/arcing halls) and
/// *reshape its cells* to the rasterized shape, exactly as if the node had
/// grown that way. Shapes claim free cells, so a shape reaching another
/// area's neighbourhood unions with it at the cell level — the outline then
/// traces one loop around both and no interior border ever exists. An area
/// whose reshaping would disconnect it or orphan a door stays organic.
pub fn build<R: Rng>(
    areas: &mut Areas,
    topology: &Topology,
    grid: &HexGrid,
    level: f64,
    hex_size: f64,
    rng: &mut R,
) -> Vec<Option<RuinShape>> {
    let n = areas.count();
    let mut out = vec![None; n];
    if level <= 0.0 {
        return out;
    }
    let mut eligible: Vec<usize> = (0..n).collect();
    let count = ((level.clamp(0.0, 1.0) * eligible.len() as f64).round() as usize)
        .min(eligible.len());
    if count == 0 {
        return out;
    }
    eligible.shuffle(rng);
    for &i in eligible.iter().take(count) {
        let shape = if topology.is_corridor[i] {
            fit_hall(&areas.cells[i], hex_size, rng)
        } else {
            Some(fit(&areas.cells[i], hex_size, rng))
        };
        let Some(shape) = shape else { continue };
        if reshape(areas, topology, grid, i, shape, hex_size) {
            out[i] = Some(shape);
        }
    }
    out
}

/// Cell → shape map used for wall projection and decor classification.
/// Seam cells — where a reshaped area touches a *different* area — stay
/// organic: the merged throat then keeps its full cell width instead of
/// pinching down to the shapes' exact geometric intersection, and its walls
/// carry organic decoration.
pub fn ruin_cell_map(
    areas: &Areas,
    shapes: &[Option<RuinShape>],
) -> std::collections::HashMap<Hex, RuinShape> {
    let mut map = std::collections::HashMap::new();
    for (i, shape) in shapes.iter().enumerate() {
        let Some(shape) = shape else { continue };
        for &c in &areas.cells[i] {
            let seam = c
                .neighbors()
                .iter()
                .any(|n| areas.owner_of(*n).is_some_and(|o| o != i));
            if !seam {
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
    areas.replace_area(i, cells);
    true
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
        if perp(&apex).abs() > 0.8 * s
            && let Some((center, r)) = circumcircle(a, apex, b)
            && r < best * 4.0
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
