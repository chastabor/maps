//! Outline self-intersection check.
//!
//! This file also held `trace_pinch`, a seed-pinned regression test for the
//! pinched-passage report, asserting a minimum wall gap of 5px. It was calibrated
//! against the clamping-era geometry that `plans/tile-first-render.md` is replacing —
//! walls positioned by projecting a traced boundary onto a fitted shape — so it
//! measured the behaviour being removed rather than a property worth keeping. Dropped
//! with the rectangular board, which reshapes what its pinned seed generates anyway.
//! New cases get written against the tile-first geometry as issues surface.

use maps_core::tags::Tags;

fn seg_intersect(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let cross = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    let (d1, d2) = (cross(a, b, c), cross(a, b, d));
    let (d3, d4) = (cross(c, d, a), cross(c, d, b));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

#[test]
fn no_self_intersecting_loops() {
    use maps_core::{GenOptions, generate_with};
    let map = generate_with(
        9521512733245147772,
        &GenOptions {
            tags: Some(Tags::parse("medium,chamber,coral,tree,junction,dry,ruins").unwrap()),
            ruins_level: Some(0.93),
            shape_seed: Some(9767082189707533793),
            decor_seed: Some(11680530627533803770),
            name_seed: Some(3172840391344839462),
            ..GenOptions::default()
        },
    );
    for (i, area) in map.areas.cells.iter().enumerate() {
        println!(
            "area {i}: {} cells, corridor={}, ruin={:?}",
            area.len(),
            map.topology.is_corridor[i],
            map.ruins[i]
        );
    }
    let mut crossings = Vec::new();
    for (li, lp) in map.outline.iter().enumerate() {
        let n = lp.len();
        for i in 0..n {
            let (a, b) = (lp[i], lp[(i + 1) % n]);
            for j in (i + 2)..n {
                if i == 0 && j == n - 1 {
                    continue; // adjacent via wrap
                }
                let (c, d) = (lp[j], lp[(j + 1) % n]);
                if seg_intersect(a, b, c, d) {
                    crossings.push((li, i, j, a));
                }
            }
        }
    }
    for (li, i, j, p) in &crossings {
        println!(
            "  self-intersection loop {li}: seg {i} x seg {j} near ({:.0},{:.0})",
            p.0, p.1
        );
    }
    assert!(
        crossings.is_empty(),
        "{} self-intersections found",
        crossings.len()
    );
}
