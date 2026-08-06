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
    /// The span between parameters `a` and `b`, the **shorter way round** a perimeter of
    /// length `per`.
    ///
    /// The one rule for turning two wall parameters into an interval: anything a span is cut
    /// for (a doorway, a tile edge's contact) covers a small fraction of the border, so the
    /// short arc is never the ambiguous choice — and both `corridor` and `doorway` derived
    /// this independently before it had a home here.
    pub fn shorter(a: f64, b: f64, per: f64) -> Span {
        let fwd = (b - a).rem_euclid(per);
        if fwd <= per - fwd {
            Span { from: a, len: fwd }
        } else {
            Span {
                from: b,
                len: per - fwd,
            }
        }
    }

    /// Whether parameter `t` falls inside this span, on a perimeter of length `per`.
    pub fn contains(&self, t: f64, per: f64) -> bool {
        (t - self.from).rem_euclid(per) <= self.len + 1e-9
    }

    /// Whether `t` falls **strictly** inside — inside, and not at either end. The question a
    /// corner is asked: a span *ending* on a corner parameter does not turn it.
    pub fn strictly_contains(&self, t: f64, per: f64) -> bool {
        self.contains(t, per)
            && (t - self.from).rem_euclid(per) > 1e-6
            && (t - self.end(per)).rem_euclid(per) > 1e-6
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
    merge_groups(spans, per, tol)
        .into_iter()
        .map(|(s, _)| s)
        .collect()
}

/// [`merge`], keeping provenance: each merged span comes with the indices (into `spans`) of the
/// spans it absorbed.
///
/// The merge is the one place that *knows* which inputs joined which output; a caller that
/// re-derives the grouping afterwards (span containment tests against the merged result) is
/// making a second, independent decision about the same fact — the exact drift hazard this
/// crate keeps finding. Phase 3's ring gaps need the grouping to size each door from its own
/// contributing tile edges, per landing today and per room when the cross-corridor merge lands.
pub fn merge_groups(spans: &[Span], per: f64, tol: f64) -> Vec<(Span, Vec<usize>)> {
    if per <= 0.0 {
        return Vec::new();
    }
    // Flatten to linear intervals in [0, 2·per), splitting wrapped spans (both halves keep the
    // source index, and the rejoin below unions their groups back together).
    let mut iv: Vec<(f64, f64, usize)> = Vec::new();
    let mut whole: Vec<usize> = Vec::new();
    for (k, s) in spans.iter().enumerate() {
        if s.len <= 0.0 {
            continue;
        }
        if s.len >= per - 1e-9 {
            whole.push(k);
            continue;
        }
        let a = s.from.rem_euclid(per);
        let b = a + s.len;
        if b <= per {
            iv.push((a, b, k));
        } else {
            iv.push((a, per, k));
            iv.push((0.0, b - per, k));
        }
    }
    if !whole.is_empty() {
        // A whole-perimeter input absorbs everything.
        return vec![(
            Span {
                from: 0.0,
                len: per,
            },
            (0..spans.len()).filter(|&k| spans[k].len > 0.0).collect(),
        )];
    }
    if iv.is_empty() {
        return Vec::new();
    }
    iv.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64, Vec<usize>)> = vec![(iv[0].0, iv[0].1, vec![iv[0].2])];
    for &(a, b, k) in &iv[1..] {
        let last = out.last_mut().unwrap();
        if a <= last.1 + tol {
            last.1 = last.1.max(b);
            last.2.push(k);
        } else {
            out.push((a, b, vec![k]));
        }
    }
    // Rejoin across the origin if the first and last intervals touch there.
    if out.len() > 1 {
        let (first_end, last_start) = (out[0].1, out.last().unwrap().0);
        if out[0].0 <= tol && out.last().unwrap().1 >= per - tol {
            let (_, _, mut members) = out.pop().unwrap();
            members.append(&mut out[0].2);
            out[0] = (last_start - per, first_end, members);
        }
    }
    if out.len() == 1 && out[0].1 - out[0].0 >= per - tol {
        let mut members = std::mem::take(&mut out[0].2);
        members.sort_unstable();
        members.dedup();
        return vec![(
            Span {
                from: 0.0,
                len: per,
            },
            members,
        )];
    }
    out.into_iter()
        .map(|(a, b, mut members)| {
            // A wrapped input contributed both halves; one membership is enough.
            members.sort_unstable();
            members.dedup();
            (
                Span {
                    from: a.rem_euclid(per),
                    len: b - a,
                },
                members,
            )
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

/// Split a tagged wall run into the stretches that **no other room encloses**.
///
/// The counterpart to [`wall_spans`], for the other side of the same cut. Where a room's border
/// is clipped by the things that open it, a *connector's* wall is clipped by the rooms it runs
/// into: a corridor is built from the tiles the two rooms share, and those are room tiles, so its
/// walls necessarily start out running inside both rooms. That stretch is not wall — it is open
/// floor with a line drawn across it.
///
/// Each vertex carries the shape it belongs to, so a run is only ever clipped against *other*
/// rooms. Without that a fused compound's band would clip away its own two rooms' walls, which is
/// most of it.
///
/// `margin` is how far inside a room a point must be to count as enclosed. A wall legitimately
/// *touches* a border it meets — a squared seam corner lies exactly on both — so testing plain
/// containment would eat the join. One pixel is enough to separate "on the border" from "inside
/// the room".
///
/// Returns the surviving stretches in order, each with the crossing point as its new end, so the
/// renderer's per-run capsule ends where the wall really ends and the gap between two stretches
/// is the passage. Stretches of fewer than two points are dropped: a polyline needs two.
pub fn split_outside(
    run: &[(Point, RuinShape)],
    rooms: &[RuinShape],
    margin: f64,
) -> Vec<Vec<(Point, RuinShape)>> {
    // Clip against each room shrunk by `margin` rather than testing distance-to-border as we
    // go: it puts the margin into the geometry, so the crossings stay exact. A wall legitimately
    // *touches* a border it meets — a squared seam corner lies on both — and without the inset
    // that join would be eaten.
    let inner: Vec<RuinShape> = rooms.iter().map(|r| r.shrink(margin)).collect();
    // Ownership is deliberately NOT an exemption. A room's wall lies *on* its own border, so it
    // never falls inside the shrunk copy, and a stretch that does — a wall rerouted across its
    // own room to meet a corridor — is the same defect as a wall inside a neighbour.
    let enclosed = |p: Point| inner.iter().any(|r| r.contains(p));
    let mut out: Vec<Vec<(Point, RuinShape)>> = Vec::new();
    let mut cur: Vec<(Point, RuinShape)> = Vec::new();
    for w in run.windows(2) {
        let ((a, sa), (b, _)) = (w[0], w[1]);
        let lerp = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        // The endpoints are the ORIGINAL vertices, not `lerp(0)`/`lerp(1)`: the arithmetic is
        // not bit-identical, and a vertex that differs in its last bits from the next segment's
        // start is a duplicate the run did not have before. That alone moved every digest.
        let at = |t: f64| {
            if t <= 0.0 {
                a
            } else if t >= 1.0 {
                b
            } else {
                lerp(t)
            }
        };
        // Every parameter at which this segment enters or leaves a room, exactly. Between two
        // consecutive breakpoints the segment is wholly inside or wholly outside, so one
        // midpoint test classifies the whole interval — no sampling, and nothing narrow can
        // hide between samples. This is what lets a wall crossing the 6px shoulder gap between
        // two fused rects survive: both its endpoints are enclosed, but the middle is not.
        let mut ts: Vec<f64> = vec![0.0, 1.0];
        for r in &inner {
            ts.extend(r.segment_crossings(a, b));
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        for pair in ts.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let (p0, p1) = (at(t0), at(t1));
            if enclosed(at(0.5 * (t0 + t1))) {
                // Inside a room: not wall. Close whatever was open.
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                continue;
            }
            if cur.last().map(|&(p, _)| p) != Some(p0) {
                cur.push((p0, sa));
            }
            cur.push((p1, sa));
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}
