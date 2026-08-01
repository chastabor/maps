//! `clip` — the wall is the border minus its openings.
//!
//! Cases chosen so the answer is arithmetic rather than whatever the code returns: unit-ish
//! circles whose overlap subtends a round angle, plus every degenerate arrangement that must
//! yield "nothing to cut" rather than a near-miss (separate, nested, tangent, hall).
//!
//! The wrap cases are the ones worth having. A span is stored as `(from, len)` on a cyclic
//! perimeter, so an opening that straddles the parameter origin is the arrangement where an
//! off-by-one shows up, and on a circle the origin is at angle 0 — the +x axis — which is exactly
//! where a left-right fused pair puts its seam.

use maps_core::clip::{Span, merge, opening_span, openings_from, wall_spans};
use maps_core::ruins::RuinShape as R;
use std::f64::consts::TAU;

const TOL: f64 = 0.05;

fn circle(cx: f64, cy: f64, r: f64) -> R {
    R::Circle { cx, cy, r }
}

fn rect(cx: f64, cy: f64, hw: f64, hh: f64) -> R {
    R::Rect { cx, cy, hw, hh }
}

/// Total arc a set of spans covers.
fn total(spans: &[Span]) -> f64 {
    spans.iter().map(|s| s.len).sum()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// --- opening_span ----------------------------------------------------------------------------

#[test]
fn two_circles_overlapping_cut_a_known_arc() {
    // Centres 100 apart, both r=100: the crossings sit at ±60° from the centre line, so each
    // circle's arc inside the other subtends 2·60° = 120° = a third of its circumference.
    let (a, b) = (circle(0.0, 0.0, 100.0), circle(100.0, 0.0, 100.0));
    let s = opening_span(&a, &b).expect("overlapping circles cut an arc");
    assert!(
        close(s.len, TAU * 100.0 / 3.0),
        "expected a third of the circumference, got {} of {}",
        s.len,
        TAU * 100.0
    );
    // Symmetric pair, so b's opening is the same size.
    let t = opening_span(&b, &a).expect("and the other way round");
    assert!(close(t.len, s.len), "{} vs {}", t.len, s.len);
}

#[test]
fn the_opening_is_the_arc_inside_the_partner() {
    // a is left of b, so a's opening must be its RIGHT side: the span's midpoint has x > 0.
    let (a, b) = (circle(0.0, 0.0, 100.0), circle(100.0, 0.0, 100.0));
    let s = opening_span(&a, &b).unwrap();
    let mid = a.wall_point((s.from + s.len / 2.0).rem_euclid(TAU * 100.0));
    assert!(mid.0 > 0.0, "opening midpoint {mid:?} is on the far side");
    assert!(b.contains(mid), "opening midpoint {mid:?} is not inside b");
    // And the wall that remains is on the left.
    let w = wall_spans(&a, &[s], TOL);
    assert_eq!(w.len(), 1, "one opening leaves one wall span: {w:?}");
    let wmid = a.wall_point((w[0].from + w[0].len / 2.0).rem_euclid(TAU * 100.0));
    assert!(wmid.0 < 0.0, "wall midpoint {wmid:?} is on the wrong side");
}

#[test]
fn nothing_to_cut_yields_none() {
    let a = circle(0.0, 0.0, 50.0);
    for (why, other) in [
        ("separate", circle(500.0, 0.0, 50.0)),
        ("nested", circle(0.0, 0.0, 10.0)),
        ("containing", circle(0.0, 0.0, 500.0)),
        ("concentric equal", circle(0.0, 0.0, 50.0)),
        ("externally tangent", circle(100.0, 0.0, 50.0)),
        (
            "a hall has no closed border",
            R::StraightHall {
                ax: -100.0,
                ay: 0.0,
                bx: 100.0,
                by: 0.0,
                hw: 6.0,
            },
        ),
    ] {
        assert!(
            opening_span(&a, &other).is_none(),
            "{why}: expected no opening, got {:?}",
            opening_span(&a, &other)
        );
    }
}

#[test]
fn a_hall_has_no_wall_spans() {
    let hall = R::StraightHall {
        ax: 0.0,
        ay: 0.0,
        bx: 50.0,
        by: 0.0,
        hw: 6.0,
    };
    assert!(wall_spans(&hall, &[], TOL).is_empty());
}

#[test]
fn circle_and_rect_cut_each_other() {
    // A rect overlapping the circle's right side.
    let c = circle(0.0, 0.0, 100.0);
    let r = rect(120.0, 0.0, 60.0, 40.0);
    let cs = opening_span(&c, &r).expect("the rect cuts the circle");
    let rs = opening_span(&r, &c).expect("and the circle cuts the rect");
    // Each opening's midpoint lies inside the other, and each is a proper part of its border.
    for (sh, other, s) in [(&c, &r, cs), (&r, &c, rs)] {
        let per = sh.perimeter().unwrap();
        let mid = sh.wall_point((s.from + s.len / 2.0).rem_euclid(per));
        assert!(
            other.contains(mid),
            "midpoint {mid:?} not inside the partner"
        );
        assert!(s.len > 0.0 && s.len < per, "span {s:?} is not a proper arc");
    }
}

// --- merge and complement --------------------------------------------------------------------

#[test]
fn no_openings_leaves_the_whole_border() {
    let a = circle(0.0, 0.0, 100.0);
    let w = wall_spans(&a, &[], TOL);
    assert_eq!(w.len(), 1);
    assert!(close(w[0].len, TAU * 100.0), "{:?}", w[0]);
}

#[test]
fn openings_covering_everything_leave_no_wall() {
    let a = circle(0.0, 0.0, 100.0);
    let per = TAU * 100.0;
    let both = [
        Span {
            from: 0.0,
            len: per / 2.0,
        },
        Span {
            from: per / 2.0,
            len: per / 2.0,
        },
    ];
    assert!(
        wall_spans(&a, &both, TOL).is_empty(),
        "two half-borders cover the whole thing"
    );
}

#[test]
fn two_openings_leave_two_walls_summing_to_the_remainder() {
    let a = circle(0.0, 0.0, 100.0);
    let per = TAU * 100.0;
    let cuts = [
        Span {
            from: 0.1 * per,
            len: 0.2 * per,
        },
        Span {
            from: 0.5 * per,
            len: 0.1 * per,
        },
    ];
    let w = wall_spans(&a, &cuts, TOL);
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        close(total(&w), 0.7 * per),
        "walls total {}, expected {}",
        total(&w),
        0.7 * per
    );
    // No wall span may overlap an opening.
    for s in &w {
        let mid = (s.from + s.len / 2.0).rem_euclid(per);
        for c in &cuts {
            assert!(
                !c.contains(mid, per),
                "wall {s:?} sits inside opening {c:?}"
            );
        }
    }
}

#[test]
fn an_opening_across_the_parameter_origin_is_one_span() {
    // The +x axis is parameter 0 on a circle, and a left-right fused pair seams right there.
    let per = TAU * 100.0;
    let wrapped = Span {
        from: 0.9 * per,
        len: 0.2 * per,
    };
    let m = merge(&[wrapped], per, TOL);
    assert_eq!(m.len(), 1, "a wrapped opening must not split in two: {m:?}");
    assert!(close(m[0].len, 0.2 * per), "{:?}", m[0]);

    let a = circle(0.0, 0.0, 100.0);
    let w = wall_spans(&a, &[wrapped], TOL);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(close(w[0].len, 0.8 * per), "{:?}", w[0]);
}

#[test]
fn overlapping_openings_merge() {
    let per = 100.0;
    let m = merge(
        &[
            Span {
                from: 10.0,
                len: 20.0,
            },
            Span {
                from: 25.0,
                len: 20.0,
            },
        ],
        per,
        TOL,
    );
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(
        close(m[0].from, 10.0) && close(m[0].len, 35.0),
        "{:?}",
        m[0]
    );
}

#[test]
fn openings_touching_across_the_origin_merge_into_one() {
    let per = 100.0;
    let m = merge(
        &[
            Span {
                from: 90.0,
                len: 10.0,
            },
            Span {
                from: 0.0,
                len: 10.0,
            },
        ],
        per,
        TOL,
    );
    assert_eq!(m.len(), 1, "these are one arc through the origin: {m:?}");
    assert!(close(m[0].len, 20.0), "{:?}", m[0]);
}

#[test]
fn a_slither_of_wall_narrower_than_the_tolerance_is_dropped() {
    let a = circle(0.0, 0.0, 100.0);
    let per = TAU * 100.0;
    // Two openings leaving a 0.01px stub between them.
    let cuts = [
        Span {
            from: 0.0,
            len: per / 2.0,
        },
        Span {
            from: per / 2.0 + 0.01,
            len: per / 2.0 - 0.02,
        },
    ];
    let w = wall_spans(&a, &cuts, TOL);
    assert!(
        w.iter().all(|s| s.len > TOL),
        "a sub-tolerance wall stub survived: {w:?}"
    );
}

// --- the whole-border identity ---------------------------------------------------------------

#[test]
fn wall_plus_openings_is_the_whole_border() {
    // The property the construction rests on, over an arrangement with several partners: a room
    // is exactly its wall plus its openings, with nothing counted twice.
    let a = circle(0.0, 0.0, 100.0);
    let per = a.perimeter().unwrap();
    let others = [
        circle(150.0, 0.0, 80.0),
        circle(-140.0, 40.0, 70.0),
        rect(20.0, -130.0, 50.0, 50.0),
        circle(600.0, 600.0, 40.0), // far away: contributes nothing
    ];
    let cuts = openings_from(&a, &others);
    assert_eq!(
        cuts.len(),
        3,
        "three neighbours overlap, one does not: {cuts:?}"
    );
    let merged = merge(&cuts, per, TOL);
    let walls = wall_spans(&a, &cuts, TOL);
    assert!(
        close(total(&merged) + total(&walls), per),
        "wall {} + openings {} = {}, expected {per}",
        total(&walls),
        total(&merged),
        total(&merged) + total(&walls)
    );
    // Sampling agrees: a point is on the wall iff no neighbour contains it.
    let n = 2000;
    for i in 0..n {
        let t = per * i as f64 / n as f64;
        let p = a.wall_point(t);
        let covered = others.iter().any(|o| o.contains(p));
        let on_wall = walls.iter().any(|s| s.contains(t, per));
        // Skip the immediate neighbourhood of a boundary, where the two disagree only by
        // which side of the crossing a sample lands on.
        let near_edge = merged
            .iter()
            .flat_map(|s| [s.from, s.end(per)])
            .any(|e| (t - e).rem_euclid(per).min((e - t).rem_euclid(per)) < 0.5);
        if !near_edge {
            assert_eq!(
                on_wall, !covered,
                "at t={t:.2} ({p:?}): on_wall={on_wall} but covered={covered}"
            );
        }
    }
}

#[test]
fn a_rects_corners_are_not_special() {
    // A rect's parameter has seams at its four corners; an opening spanning one must stay one
    // span. This rect is cut by a circle centred on its top-right corner.
    let r = rect(0.0, 0.0, 60.0, 40.0);
    let per = r.perimeter().unwrap();
    let c = circle(60.0, -40.0, 25.0);
    let s = opening_span(&r, &c).expect("the circle cuts the corner");
    let w = wall_spans(&r, &[s], TOL);
    assert_eq!(w.len(), 1, "one opening leaves one wall: {w:?}");
    assert!(
        close(s.len + w[0].len, per),
        "{} + {} != {per}",
        s.len,
        w[0].len
    );
    // The opening really does straddle the corner: it contains points on both incident edges.
    let mid_top = r.wall_point((s.from + s.len * 0.25).rem_euclid(per));
    let mid_right = r.wall_point((s.from + s.len * 0.75).rem_euclid(per));
    assert!(
        c.contains(mid_top) && c.contains(mid_right),
        "{mid_top:?} / {mid_right:?} should both be inside the cutting circle"
    );
}

#[test]
fn half_turn_symmetry_gives_mirrored_spans() {
    // Rotating the whole arrangement by π must rotate the spans by half the perimeter — the
    // symmetry `derive_shape` relies on, checked on the clip rather than assumed.
    let per = TAU * 100.0;
    let a = circle(0.0, 0.0, 100.0);
    let right = opening_span(&a, &circle(150.0, 0.0, 80.0)).unwrap();
    let left = opening_span(&a, &circle(-150.0, 0.0, 80.0)).unwrap();
    assert!(close(right.len, left.len), "{right:?} vs {left:?}");
    let shifted = (right.from + per / 2.0).rem_euclid(per);
    let d = (shifted - left.from).rem_euclid(per);
    assert!(
        d.min(per - d) < 1e-6,
        "{right:?} rotated by half a turn should be {left:?}"
    );
}
