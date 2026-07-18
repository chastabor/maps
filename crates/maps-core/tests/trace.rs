//! Regression test for the pinched-passage report (hall projection folding
//! opposite walls onto the same circle). Diagnostic output with:
//! cargo test -p maps-core --test trace -- --nocapture

use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

#[test]
fn trace_pinch() {
    let opts = |ruins: &str| GenOptions {
        tags: Some(Tags::parse(&format!("medium,hub,coral,tree,junction,dry,{ruins}")).unwrap()),
        shape_seed: Some(9767082189707533793),
        decor_seed: Some(11680530627533803770),
        name_seed: Some(3172840391344839462),
        ..GenOptions::default()
    };

    for ruins in ["ruins", "organic"] {
        let map = generate_with(9521512733245147772, &opts(ruins));
        println!("=== {ruins} variant: {} areas ===", map.areas.count());
        for (i, area) in map.areas.cells.iter().enumerate() {
            println!(
                "  area {i}: {} cells, corridor={}, ruin={:?}",
                area.len(),
                map.topology.is_corridor[i],
                map.ruins[i]
            );
        }
        if ruins == "ruins" {
            let (cx, cy) = (-148.95636945092343, -13.99999999999999);
            for &c in &map.areas.cells[2] {
                let (x, y) = c.center(12.0);
                let d = (x - cx).hypot(y - cy);
                println!(
                    "    corridor cell {:?} centre ({x:.0},{y:.0}) d_from_arc_centre {d:.1}",
                    c
                );
            }
            for (li, lp) in map.outline.iter().enumerate() {
                let near: Vec<f64> = lp
                    .iter()
                    .map(|p| (p.0 - cx).hypot(p.1 - cy))
                    .filter(|d| *d < 60.0)
                    .collect();
                if !near.is_empty() {
                    let min = near.iter().cloned().fold(f64::MAX, f64::min);
                    let max = near.iter().cloned().fold(f64::MIN, f64::max);
                    println!(
                        "    loop {li}: {} pts within 60px of arc centre, radius {min:.1}..{max:.1}",
                        near.len()
                    );
                }
            }
        }
        // Pinch scan: boundary points close in space but not neighbours
        // along the boundary = opposite walls nearly touching. Checks both
        // within a loop and across loops (necks between a loop and a hole).
        println!("  loops: {:?}", map.outline.iter().map(|l| l.len()).collect::<Vec<_>>());
        let mut worst: Vec<(f64, (f64, f64), usize, usize)> = Vec::new();
        for (li, lp) in map.outline.iter().enumerate() {
            let n = lp.len();
            for i in 0..n {
                for j in (i + 20)..n {
                    if n - (j - i) < 20 {
                        continue; // wrap-around neighbours
                    }
                    let d = (lp[i].0 - lp[j].0).hypot(lp[i].1 - lp[j].1);
                    if d < 13.0 {
                        worst.push((d, lp[i], li, li));
                    }
                }
            }
        }
        for (a, la) in map.outline.iter().enumerate() {
            for (b, lb) in map.outline.iter().enumerate().skip(a + 1) {
                for p in la {
                    for q in lb {
                        let d = (p.0 - q.0).hypot(p.1 - q.1);
                        if d < 13.0 {
                            worst.push((d, *p, a, b));
                        }
                    }
                }
            }
        }
        worst.sort_by(|a, b| a.0.total_cmp(&b.0));
        worst.dedup_by(|a, b| (a.1.0 - b.1.0).hypot(a.1.1 - b.1.1) < 15.0);
        println!("  necks (gap < 13px): {}", worst.len());
        for (d, p, la, lb) in worst.iter().take(10) {
            println!("    gap {d:.2}px at ({:.0},{:.0}) loops {la}/{lb}", p.0, p.1);
        }
        // The organic baseline for this seed bottoms out around 7px (coral
        // corner-necks). Ruin projection must not squeeze walls below the
        // point where strokes seal the passage shut.
        if let Some((d, p, _, _)) = worst.first() {
            assert!(
                *d > 5.0,
                "{ruins}: wall gap {d:.2}px at ({:.0},{:.0}) — passage pinched shut",
                p.0,
                p.1
            );
        }
    }
}
