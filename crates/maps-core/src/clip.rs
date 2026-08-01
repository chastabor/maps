//! Clipping: a shaped room's wall is **its own border minus the spans its openings occupy**.
//!
//! This is the construction `plans/tile-first-render.md` phase 2 calls for, as pure geometry over
//! shapes. The pipeline it replaces works the other way round: it traces the floor's cell
//! boundary, smooths it, and projects each vertex onto the room's shape, so the wall is only ever
//! as good as the raster it came from and every opening has to be reconciled in pixel space
//! afterwards (jamb snapping, the tangency guard, the support clamp). Here the border is drawn
//! from the shape and the openings are *cut out of it*, so an opening's edges are exactly where
//! the two geometries meet and there is nothing to reconcile.
//!
//! Everything is expressed as [`Span`]s in a shape's own wall parameter — arc length for a
//! circle, perimeter distance for a rect — because that is the one coordinate in which "remove
//! this stretch of wall" is a subtraction rather than a search. [`RuinShape::wall_point`] turns a
//! parameter back into a point, and [`crate::outline`] already walks a parameter range
//! (`wall_walk`), so a span is directly renderable.
//!
//! Two properties make the subtraction well defined, both asserted over real maps by
//! `tests/openings.rs`:
//!
//! - a fused pair's borders cross in **two points or none**, never several, so "the arc inside
//!   the partner" is a single interval;
//! - a connector's wall crosses a room's border **at most once**, so a throat is one cut.

use crate::geom::Point;
use crate::ruins::RuinShape;

/// A stretch of a shape's perimeter: `len` of arc starting at parameter `from`.
///
/// `from` is taken modulo the perimeter and `len` is non-negative and measured in the direction
/// of increasing parameter, so a span may wrap past the parameter origin. `len == perimeter`
/// means the whole border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub from: f64,
    pub len: f64,
}

impl Span {
    /// Whether parameter `t` falls inside this span, on a perimeter of length `per`.
    pub fn contains(&self, t: f64, per: f64) -> bool {
        (t - self.from).rem_euclid(per) <= self.len + 1e-9
    }

    /// The parameter this span ends at.
    pub fn end(&self, per: f64) -> f64 {
        (self.from + self.len).rem_euclid(per)
    }
}

/// Distinct crossings of two borders.
///
/// Deduped because a rect corner lying on the other shape's edge is reported once by each edge
/// incident to it, and a caller counting crossings would see three where there are two.
fn crossings(a: &RuinShape, b: &RuinShape) -> Vec<Point> {
    let mut xs: Vec<Point> = Vec::new();
    for p in a.border_crossings(b) {
        if !xs.iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < 1e-6) {
            xs.push(p);
        }
    }
    xs
}

/// The span of `shape`'s border that lies **inside** `other` — the opening `other` cuts into it.
///
/// `None` when there is nothing to cut rather than when the computation fails, which is the
/// distinction a caller needs:
///
/// - either shape has no closed border (a hall or a connector trapezoid, whose `perimeter()` is
///   `None`) — those meet a room along their own walls, not by enclosing an arc of it;
/// - the borders do not cross in exactly two points. Zero means they never meet, so the wall is
///   whole. More than two means several disjoint spans, which no fused pair produces (measured
///   over 200 seeds) but two rects crossing like a `+` would;
/// - one border lies wholly inside the other, so there is no arc left outside to be wall.
///
/// Which of the two arcs between the crossings is the opening is decided by asking whether its
/// own midpoint is inside `other`. That has one answer, needs no tolerance, and does not depend
/// on the order the crossings came back in.
pub fn opening_span(shape: &RuinShape, other: &RuinShape) -> Option<Span> {
    let per = shape.perimeter()?;
    other.perimeter()?;
    let xs = crossings(shape, other);
    if xs.len() != 2 {
        return None;
    }
    let (t0, t1) = (shape.wall_param(xs[0]), shape.wall_param(xs[1]));
    [
        Span {
            from: t0,
            len: (t1 - t0).rem_euclid(per),
        },
        Span {
            from: t1,
            len: (t0 - t1).rem_euclid(per),
        },
    ]
    .into_iter()
    .find(|s| {
        s.len > 1e-9 && other.contains(shape.wall_point((s.from + s.len / 2.0).rem_euclid(per)))
    })
}

/// Merge overlapping and touching spans on a perimeter of length `per`, returning them sorted by
/// start. A span that wraps the parameter origin is split, merged, and rejoined, so the result is
/// the same set of arcs whichever side of the origin an opening happens to land on.
///
/// `tol` closes gaps narrower than itself: two openings a hair apart would otherwise leave a
/// slither of wall between them that is shorter than a rendered line join.
pub fn merge(spans: &[Span], per: f64, tol: f64) -> Vec<Span> {
    if per <= 0.0 {
        return Vec::new();
    }
    // Flatten to linear intervals in [0, 2·per), splitting wrapped spans.
    let mut iv: Vec<(f64, f64)> = Vec::new();
    for s in spans {
        if s.len <= 0.0 {
            continue;
        }
        if s.len >= per - 1e-9 {
            return vec![Span {
                from: 0.0,
                len: per,
            }];
        }
        let a = s.from.rem_euclid(per);
        let b = a + s.len;
        if b <= per {
            iv.push((a, b));
        } else {
            iv.push((a, per));
            iv.push((0.0, b - per));
        }
    }
    if iv.is_empty() {
        return Vec::new();
    }
    iv.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64)> = vec![iv[0]];
    for &(a, b) in &iv[1..] {
        let last = out.last_mut().unwrap();
        if a <= last.1 + tol {
            last.1 = last.1.max(b);
        } else {
            out.push((a, b));
        }
    }
    // Rejoin across the origin if the first and last intervals touch there.
    if out.len() > 1 {
        let (first, last) = (out[0], *out.last().unwrap());
        if first.0 <= tol && last.1 >= per - tol {
            out.pop();
            out[0] = (last.0 - per, first.1);
        }
    }
    if out.len() == 1 && out[0].1 - out[0].0 >= per - tol {
        return vec![Span {
            from: 0.0,
            len: per,
        }];
    }
    out.into_iter()
        .map(|(a, b)| Span {
            from: a.rem_euclid(per),
            len: b - a,
        })
        .collect()
}

/// The wall: `shape`'s border with every opening removed.
///
/// The complement of the merged openings, as the spans that remain. Empty when the openings cover
/// the whole border (a room swallowed by its fused partner has no wall of its own); the whole
/// perimeter as one span when there are no openings at all.
///
/// `tol` drops wall slithers shorter than itself as well as closing narrow gaps between openings
/// — a 0.2px stub of wall between two doors is a rendering artefact, not a wall.
pub fn wall_spans(shape: &RuinShape, openings: &[Span], tol: f64) -> Vec<Span> {
    let Some(per) = shape.perimeter() else {
        return Vec::new();
    };
    let cut = merge(openings, per, tol);
    if cut.is_empty() {
        return vec![Span {
            from: 0.0,
            len: per,
        }];
    }
    if cut.len() == 1 && cut[0].len >= per - tol {
        return Vec::new();
    }
    // Each remaining stretch runs from one opening's end to the next opening's start.
    let mut out = Vec::new();
    for (i, c) in cut.iter().enumerate() {
        let next = cut[(i + 1) % cut.len()];
        let from = c.end(per);
        let len = (next.from - from).rem_euclid(per);
        if len > tol {
            out.push(Span { from, len });
        }
    }
    out
}

/// Every opening a set of neighbouring shapes cuts into `shape`, unmerged.
///
/// A convenience over [`opening_span`] for the common caller: the neighbours that do not cut an
/// arc simply contribute nothing.
pub fn openings_from(shape: &RuinShape, others: &[RuinShape]) -> Vec<Span> {
    others
        .iter()
        .filter(|o| *o != shape)
        .filter_map(|o| opening_span(shape, o))
        .collect()
}
