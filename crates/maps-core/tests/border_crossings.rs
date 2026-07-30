//! `RuinShape::border_crossings` — the geometry phase 2a clips a fused seam with.
//!
//! Cases chosen so the answer is known independently of the code: symmetric overlaps whose
//! crossings fall on round numbers, plus the degenerate arrangements (separate, nested,
//! concentric, tangent) that must yield nothing usable rather than a near-miss point.

use maps_core::ruins::RuinShape as R;

/// Crossings sorted for comparison, since the order is an implementation detail.
fn sorted(sh: &R, other: &R) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = sh
        .border_crossings(other)
        .into_iter()
        .map(|(x, y)| ((x * 1e6).round() / 1e6, (y * 1e6).round() / 1e6))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    v
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

#[test]
fn two_circles_cross_twice_symmetrically() {
    // Radius 5 at x=0 and x=6: crossings at x=3, y=±4 (3-4-5 triangle).
    let a = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 5.0,
    };
    let b = R::Circle {
        cx: 6.0,
        cy: 0.0,
        r: 5.0,
    };
    let v = sorted(&a, &b);
    assert_eq!(v.len(), 2, "got {v:?}");
    assert!(
        near(v[0], (3.0, -4.0)) && near(v[1], (3.0, 4.0)),
        "got {v:?}"
    );
    // Symmetric in the argument order.
    assert_eq!(sorted(&b, &a), v);
}

#[test]
fn circles_that_do_not_cross_yield_nothing() {
    let unit = |cx: f64, r: f64| R::Circle { cx, cy: 0.0, r };
    // Separate, nested, concentric-equal, and internally tangent.
    for (a, b, why) in [
        (unit(0.0, 2.0), unit(10.0, 2.0), "separate"),
        (unit(0.0, 10.0), unit(1.0, 2.0), "nested"),
        (unit(0.0, 5.0), unit(0.0, 5.0), "concentric identical"),
        (unit(0.0, 5.0), unit(3.0, 2.0), "internally tangent"),
    ] {
        assert!(
            a.border_crossings(&b).is_empty(),
            "{why}: expected no crossings, got {:?}",
            a.border_crossings(&b)
        );
    }
}

#[test]
fn circle_crosses_a_rect_edge() {
    // Circle radius 5 centred on the rect's right edge at x=10: it cuts that edge at
    // y=±5 (both inside the edge's span, which runs y=-8..8) and the top and bottom
    // edges once each, at x = 10 ± sqrt(25-64) → none. So exactly two crossings.
    let rect = R::Rect {
        cx: 0.0,
        cy: 0.0,
        hw: 10.0,
        hh: 8.0,
    };
    let circle = R::Circle {
        cx: 10.0,
        cy: 0.0,
        r: 5.0,
    };
    let v = sorted(&circle, &rect);
    assert_eq!(v.len(), 2, "got {v:?}");
    assert!(
        near(v[0], (10.0, -5.0)) && near(v[1], (10.0, 5.0)),
        "got {v:?}"
    );
    assert_eq!(sorted(&rect, &circle), v);
}

#[test]
fn circle_wholly_inside_a_rect_yields_nothing() {
    let rect = R::Rect {
        cx: 0.0,
        cy: 0.0,
        hw: 10.0,
        hh: 8.0,
    };
    let circle = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 3.0,
    };
    assert!(circle.border_crossings(&rect).is_empty());
    assert!(rect.border_crossings(&circle).is_empty());
}

#[test]
fn overlapping_rects_cross_at_two_corners() {
    // `a` spans x=-10..10, y=-4..4; `b` spans x=0..20, y=0..8. The overlap box is
    // x=0..10, y=0..4, and the borders cross at the two corners of that box which lie on
    // the *other* rect's edge rather than on a shared one: a's y=4 edge meets b's x=0 edge
    // at (0,4), and a's x=10 edge meets b's y=0 edge at (10,0). The other two corners of
    // the box, (0,0) and (10,4), are interior to one rect or the other.
    let a = R::Rect {
        cx: 0.0,
        cy: 0.0,
        hw: 10.0,
        hh: 4.0,
    };
    let b = R::Rect {
        cx: 10.0,
        cy: 4.0,
        hw: 10.0,
        hh: 4.0,
    };
    let v = sorted(&a, &b);
    // Corner-on-edge contact can be reported by both incident edges, so dedupe.
    let mut uniq: Vec<(f64, f64)> = Vec::new();
    for p in v {
        if !uniq.iter().any(|q| near(*q, p)) {
            uniq.push(p);
        }
    }
    assert_eq!(uniq.len(), 2, "got {uniq:?}");
    assert!(uniq.iter().any(|p| near(*p, (0.0, 4.0))), "got {uniq:?}");
    assert!(uniq.iter().any(|p| near(*p, (10.0, 0.0))), "got {uniq:?}");
}

#[test]
fn halls_and_hex_cells_report_nothing() {
    // Not "no crossings exist" but "this does not answer for these" — the caller must
    // fall back to its previous endpoint rather than treat empty as geometry.
    let circle = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 5.0,
    };
    let hall = R::StraightHall {
        ax: -10.0,
        ay: 0.0,
        bx: 10.0,
        by: 0.0,
        hw: 3.0,
    };
    let cell = R::HexCell {
        cx: 0.0,
        cy: 0.0,
        s: 6.0,
    };
    assert!(circle.border_crossings(&hall).is_empty());
    assert!(hall.border_crossings(&circle).is_empty());
    assert!(circle.border_crossings(&cell).is_empty());
}

#[test]
fn a_crossing_lies_on_both_borders() {
    // The property that matters downstream: whatever comes back must sit on both walls,
    // so `wall_param` on either shape lands at the same place.
    let cases: Vec<(R, R)> = vec![
        (
            R::Circle {
                cx: 0.0,
                cy: 0.0,
                r: 31.75,
            },
            R::Circle {
                cx: 40.0,
                cy: 12.0,
                r: 52.3,
            },
        ),
        (
            R::Circle {
                cx: 5.0,
                cy: -3.0,
                r: 25.6,
            },
            R::Rect {
                cx: 30.0,
                cy: 4.0,
                hw: 20.8,
                hh: 18.0,
            },
        ),
        (
            R::Rect {
                cx: 0.0,
                cy: 0.0,
                hw: 20.8,
                hh: 30.0,
            },
            R::Rect {
                cx: 35.0,
                cy: 18.0,
                hw: 20.8,
                hh: 18.0,
            },
        ),
    ];
    for (a, b) in cases {
        let v = a.border_crossings(&b);
        assert!(!v.is_empty(), "expected crossings for {a:?} x {b:?}");
        for p in v {
            assert!(
                a.wall_dist(p) < 1e-6,
                "{p:?} is {:.9}px off {a:?}",
                a.wall_dist(p)
            );
            assert!(
                b.wall_dist(p) < 1e-6,
                "{p:?} is {:.9}px off {b:?}",
                b.wall_dist(p)
            );
        }
    }
}

// --- compound_wall: the fused pair's wall, built by clipping each border against the other ---

use maps_core::outline::compound_wall;

/// Does segment `a→b` properly cross `c→d`?
fn crosses(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let cr = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    let (d1, d2) = (cr(a, b, c), cr(a, b, d));
    let (d3, d4) = (cr(c, d, a), cr(c, d, b));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

/// Every property the construction has to hold, checked together so a case cannot pass by
/// satisfying one and breaking another.
fn assert_well_formed(a: &R, b: &R, run: &[((f64, f64), R)], why: &str) {
    assert!(run.len() > 3, "{why}: run too short ({})", run.len());
    // Closed.
    assert!(
        near(run[0].0, run[run.len() - 1].0),
        "{why}: not closed — starts {:?} ends {:?}",
        run[0].0,
        run[run.len() - 1].0
    );
    // Every vertex lies on the border it is tagged with, and none is strictly inside the
    // other shape — that is what "clipped against the other" means.
    for &(p, sh) in run {
        assert!(
            sh.wall_dist(p) < 0.15,
            "{why}: {p:?} is {:.3}px off the shape it is tagged with",
            sh.wall_dist(p)
        );
        let other = if sh == *a { b } else { a };
        let depth = if other.contains(p) {
            other.wall_dist(p)
        } else {
            0.0
        };
        assert!(
            depth < 0.15,
            "{why}: {p:?} lies {depth:.3}px inside the other shape"
        );
    }
    // Both shapes actually contribute.
    assert!(
        run.iter().any(|&(_, sh)| sh == *a) && run.iter().any(|&(_, sh)| sh == *b),
        "{why}: only one shape contributed"
    );
    // Simple: no two non-adjacent edges cross.
    let n = run.len() - 1; // last repeats the first
    for i in 0..n {
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue; // shares the closing vertex
            }
            assert!(
                !crosses(run[i].0, run[i + 1].0, run[j].0, run[j + 1].0),
                "{why}: self-intersects near {:?} and {:?}",
                run[i].0,
                run[j].0
            );
        }
    }
}

#[test]
fn two_overlapping_circles_give_a_closed_clipped_wall() {
    let a = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 31.75,
    };
    let b = R::Circle {
        cx: 45.0,
        cy: 0.0,
        r: 31.75,
    };
    let run = compound_wall(a, b, 12.0).expect("overlapping circles should build a wall");
    assert_well_formed(&a, &b, &run, "circle+circle");
}

#[test]
fn circle_fused_to_a_rect_builds_a_wall() {
    let a = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 25.6,
    };
    let b = R::Rect {
        cx: 34.0,
        cy: 6.0,
        hw: 20.8,
        hh: 18.0,
    };
    let run = compound_wall(a, b, 12.0).expect("circle+rect should build a wall");
    assert_well_formed(&a, &b, &run, "circle+rect");
    // Order must not matter to well-formedness.
    let rev = compound_wall(b, a, 12.0).expect("rect+circle should build a wall");
    assert_well_formed(&b, &a, &rev, "rect+circle");
}

#[test]
fn two_overlapping_rects_build_a_wall() {
    let a = R::Rect {
        cx: 0.0,
        cy: 0.0,
        hw: 20.8,
        hh: 30.0,
    };
    let b = R::Rect {
        cx: 35.0,
        cy: 18.0,
        hw: 20.8,
        hh: 18.0,
    };
    let run = compound_wall(a, b, 12.0).expect("overlapping rects should build a wall");
    assert_well_formed(&a, &b, &run, "rect+rect");
}

#[test]
fn the_wall_encloses_more_than_either_room_alone() {
    // The union is bigger than either part: a sanity check that the OUTSIDE arcs were taken
    // and not the inside ones, which would enclose only the lens where they overlap.
    let a = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 30.0,
    };
    let b = R::Circle {
        cx: 40.0,
        cy: 0.0,
        r: 30.0,
    };
    let run = compound_wall(a, b, 12.0).unwrap();
    let area = {
        let mut acc = 0.0;
        for i in 0..run.len() - 1 {
            acc += run[i].0.0 * run[i + 1].0.1 - run[i + 1].0.0 * run[i].0.1;
        }
        (acc / 2.0).abs()
    };
    let one = std::f64::consts::PI * 30.0 * 30.0;
    assert!(
        area > one * 1.2,
        "union area {area:.0} should exceed one circle's {one:.0} by a clear margin"
    );
}

#[test]
fn shapes_this_construction_does_not_answer_for() {
    let circle = R::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 30.0,
    };
    for (a, b, why) in [
        (
            circle,
            R::Circle {
                cx: 200.0,
                cy: 0.0,
                r: 10.0,
            },
            "separate — no crossings",
        ),
        (
            circle,
            R::Circle {
                cx: 2.0,
                cy: 0.0,
                r: 5.0,
            },
            "nested — no outside arc on the inner",
        ),
        (
            circle,
            R::StraightHall {
                ax: -50.0,
                ay: 0.0,
                bx: 50.0,
                by: 0.0,
                hw: 6.0,
            },
            "hall — no closed border",
        ),
    ] {
        assert!(compound_wall(a, b, 12.0).is_none(), "{why}: expected None");
    }
}
