//! Plane geometry shared across the pipeline: the point type and the few
//! operations that were otherwise written out by hand at every call site.
//!
//! Deliberately small. There is no vector type with operator overloads — points
//! are plain `(f64, f64)` tuples everywhere in the crate, and every helper here
//! keeps the exact arithmetic (and the exact order of operations) its call sites
//! used before, so adopting one never moves a coordinate.

pub type Point = (f64, f64);

/// The point at parameter `t` along `a → b`: `a` at 0, `b` at 1. Not clamped —
/// callers that need clamping have already done it (see [`project_on_segment`]).
pub(crate) fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// The nearest point on segment `a → b` to `p`, with its parameter along the
/// segment: `0` at `a`, `1` at `b`.
///
/// Clamped to the segment, so a point past either end projects onto that end —
/// which is what every caller wants, whether it is measuring distance to a wall
/// line, finding the nearest hex edge, or clipping a wall stretch.
pub(crate) fn project_on_segment(p: Point, a: Point, b: Point) -> (f64, Point) {
    let d = (b.0 - a.0, b.1 - a.1);
    // A degenerate segment would divide by zero; the floor keeps `t` at 0 there.
    let l2 = (d.0 * d.0 + d.1 * d.1).max(1e-9);
    let t = (((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / l2).clamp(0.0, 1.0);
    (t, (a.0 + d.0 * t, a.1 + d.1 * t))
}
