use maps_core::render::{debug_svg, svg};
use maps_core::tags::Tags;
use maps_core::{CaveMap, generate};

#[test]
fn same_seed_same_map() {
    let a = generate(42, None);
    let b = generate(42, None);
    assert_eq!(debug_svg(&a), debug_svg(&b));
    assert_eq!(svg(&a), svg(&b));
}

#[test]
fn outline_loops_are_valid() {
    for seed in 0..25 {
        let map = generate(seed, None);
        assert!(!map.outline.is_empty(), "seed {seed}: no outline loops");
        for (i, lp) in map.outline.iter().enumerate() {
            assert!(lp.len() >= 12, "seed {seed}: loop {i} has only {} points", lp.len());
            // Shoelace area must be non-degenerate.
            let mut area = 0.0;
            for j in 0..lp.len() {
                let (x1, y1) = lp[j];
                let (x2, y2) = lp[(j + 1) % lp.len()];
                area += x1 * y2 - x2 * y1;
            }
            assert!(area.abs() > 1.0, "seed {seed}: loop {i} is degenerate");
        }
    }
}

#[test]
fn explicit_tags_deterministic() {
    let tags = Tags::parse("large,hub,coral").unwrap();
    let a = debug_svg(&generate(7, Some(tags.clone())));
    let b = debug_svg(&generate(7, Some(tags)));
    assert_eq!(a, b);
}

#[test]
fn areas_never_touch() {
    for seed in 0..25 {
        let map: CaveMap = generate(seed, None);
        for (i, area) in map.areas.cells.iter().enumerate() {
            for &c in area {
                for n in c.neighbors() {
                    if let Some(o) = map.areas.owner_of(n) {
                        assert_eq!(o, i, "seed {seed}: areas {i} and {o} touch at {n:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn areas_meet_minimum_size() {
    for seed in 0..25 {
        let map = generate(seed, None);
        assert!(map.areas.count() >= 1, "seed {seed} produced no areas");
        for (i, area) in map.areas.cells.iter().enumerate() {
            // Corridors may legitimately shrink below the growth minimum.
            if map.topology.is_corridor[i] {
                assert!(!area.is_empty(), "seed {seed}: corridor {i} vanished");
            } else {
                assert!(area.len() >= 4, "seed {seed}: area {i} has {} cells", area.len());
            }
        }
    }
}

#[test]
fn doors_connect_all_areas() {
    for seed in 0..30 {
        let map = generate(seed, None);
        let n = map.areas.count();
        if n <= 1 {
            continue;
        }
        let mut adj = vec![Vec::new(); n];
        for d in &map.topology.doors {
            adj[d.a].push(d.b);
            adj[d.b].push(d.a);
        }
        let mut seen = vec![false; n];
        seen[0] = true;
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            for &j in &adj[i] {
                if !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        assert!(
            seen.iter().all(|&s| s),
            "seed {seed}: door graph disconnected ({n} areas, {} doors)",
            map.topology.doors.len()
        );
    }
}

#[test]
fn tree_tag_gives_spanning_tree() {
    for seed in 0..15 {
        let map = generate(seed, Some(Tags::parse("medium,tree").unwrap()));
        assert_eq!(
            map.topology.doors.len(),
            map.areas.count() - 1,
            "seed {seed}: tree culling should leave exactly N-1 doors"
        );
    }
}

#[test]
fn doors_touch_both_areas_after_corridors() {
    for seed in 0..30 {
        let map = generate(seed, None);
        for d in &map.topology.doors {
            for side in [d.a, d.b] {
                assert!(
                    d.cell
                        .neighbors()
                        .iter()
                        .any(|n| map.areas.owner_of(*n) == Some(side)),
                    "seed {seed}: door {:?} lost contact with area {side}",
                    d.cell
                );
            }
        }
    }
}

#[test]
fn exit_tags_control_exit_count() {
    for seed in 0..15 {
        let sealed = generate(seed, Some(Tags::parse("sealed").unwrap()));
        assert!(sealed.topology.exits.is_empty(), "seed {seed}: sealed map has exits");
        let one = generate(seed, Some(Tags::parse("entrance").unwrap()));
        assert_eq!(one.topology.exits.len(), 1, "seed {seed}: entrance map needs 1 exit");
    }
}

#[test]
fn corridor_areas_stay_connected() {
    use std::collections::HashSet;
    for seed in 0..30 {
        let map = generate(seed, None);
        for (i, area) in map.areas.cells.iter().enumerate() {
            let set: HashSet<_> = area.iter().copied().collect();
            let mut seen = HashSet::from([area[0]]);
            let mut stack = vec![area[0]];
            while let Some(c) = stack.pop() {
                for n in c.neighbors() {
                    if set.contains(&n) && seen.insert(n) {
                        stack.push(n);
                    }
                }
            }
            assert_eq!(seen.len(), area.len(), "seed {seed}: area {i} is fragmented");
        }
    }
}

#[test]
fn dry_tag_means_no_water() {
    for seed in 0..12 {
        let map = generate(seed, Some(Tags::parse("dry").unwrap()));
        assert!(map.water.is_empty(), "seed {seed}: dry map has water");
    }
}

#[test]
fn wet_tag_usually_floods() {
    let with_water = (0..12)
        .filter(|&seed| !generate(seed, Some(Tags::parse("wet,large").unwrap())).water.is_empty())
        .count();
    assert!(with_water >= 8, "only {with_water}/12 wet maps have water");
}

#[test]
fn forest_mode_swaps_hatching_for_trees() {
    use maps_core::{GenOptions, Mode, generate_with};
    for seed in 0..8 {
        let forest = generate_with(
            seed,
            &GenOptions {
                mode: Mode::Forest,
                ..GenOptions::default()
            },
        );
        assert!(!forest.trees.is_empty(), "seed {seed}: forest has no trees");
        assert!(forest.hatching.is_empty(), "seed {seed}: forest has hatching");
        let cave = generate_with(seed, &GenOptions::default());
        assert!(cave.trees.is_empty(), "seed {seed}: cave has trees");
        assert!(!cave.hatching.is_empty(), "seed {seed}: cave has no hatching");
    }
}

#[test]
fn water_level_fills_from_the_lowest_basins() {
    use maps_core::{GenOptions, generate_with};
    use std::collections::HashSet;
    let at_level = |seed, level| {
        let map = generate_with(
            seed,
            &GenOptions {
                water_level: Some(level),
                ..GenOptions::default()
            },
        );
        map.water
    };
    for seed in 0..6 {
        let low = at_level(seed, 0.3);
        let high = at_level(seed, 0.55);
        let low_pts: usize = low.iter().map(|l| l.len()).sum();
        let high_pts: usize = high.iter().map(|l| l.len()).sum();
        assert!(
            high_pts >= low_pts,
            "seed {seed}: raising the level shrank the water"
        );
        assert!(at_level(seed, 0.0).is_empty(), "seed {seed}: level 0 not dry");
        // Level 1 submerges the whole floor: one water loop per floor loop,
        // covering everything the wall outline covers.
        let full = at_level(seed, 1.0);
        assert!(!full.is_empty(), "seed {seed}: level 1 not flooded");
        let full_pts: usize = full.iter().map(|l| l.len()).sum();
        assert!(full_pts >= high_pts, "seed {seed}: level 1 smaller than 0.55");
    }
    // Same seed and level twice -> identical pools (elevation is stable).
    let a: HashSet<String> = at_level(3, 0.4).iter().map(|l| format!("{l:?}")).collect();
    let b: HashSet<String> = at_level(3, 0.4).iter().map(|l| format!("{l:?}")).collect();
    assert_eq!(a, b);
}

#[test]
fn water_bands_accompany_pools() {
    use maps_core::tags::Tags;
    use maps_core::{GenOptions, generate_with};
    let mut deep_seen = 0;
    for seed in 0..6 {
        let map = generate_with(
            seed,
            &GenOptions {
                tags: Some(Tags::parse("large").unwrap()),
                water_level: Some(0.6),
                ..GenOptions::default()
            },
        );
        assert!(!map.water.is_empty(), "seed {seed}: level 0.6 left no water");
        // The mud band is a superset of the pools, so it must exist too.
        assert!(!map.mud.is_empty(), "seed {seed}: pools without mud fringe");
        if !map.deep_water.is_empty() {
            deep_seen += 1;
        }
        // A dry map has no bands at all.
        let dry = generate_with(
            seed,
            &GenOptions {
                water_level: Some(0.0),
                ..GenOptions::default()
            },
        );
        assert!(dry.mud.is_empty() && dry.deep_water.is_empty());
    }
    assert!(deep_seen >= 4, "deep water in only {deep_seen}/6 flooded maps");
}

#[test]
fn titles_are_present_and_deterministic() {
    for seed in 0..12 {
        let a = generate(seed, None);
        let b = generate(seed, None);
        assert!(!a.title.is_empty());
        assert_eq!(a.title, b.title);
    }
}

#[test]
fn parse_tags() {
    let t = Tags::parse("large,hub,coral").unwrap();
    assert_eq!(t.to_string(), "large hub coral");
    assert!(Tags::parse("bogus").is_err());
    assert_eq!(Tags::parse("").unwrap(), Tags::default());
}
