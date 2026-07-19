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
fn organic_areas_never_touch() {
    // Ruin-reshaped areas may deliberately reach a neighbour (cell-level
    // union); organic areas must still keep their one-cell buffer.
    for seed in 0..25 {
        let map: CaveMap = generate(seed, None);
        for (i, area) in map.areas.cells.iter().enumerate() {
            if map.ruins[i].is_some() {
                continue;
            }
            for &c in area {
                let _ = c;
                for n in c.neighbors() {
                    if let Some(o) = map.areas.owner_of(n) {
                        assert!(
                            o == i || map.ruins[o].is_some(),
                            "seed {seed}: organic areas {i} and {o} touch at {n:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn title_override_is_verbatim() {
    use maps_core::{GenOptions, Mode, generate_with};
    let with_title = |title: Option<&str>, mode| {
        generate_with(
            9,
            &GenOptions {
                mode,
                title: title.map(String::from),
                ..GenOptions::default()
            },
        )
    };
    let plain = with_title(None, Mode::Cave);
    let named = with_title(Some("The Test Hall"), Mode::Cave);
    assert_eq!(named.title, "The Test Hall");
    assert_eq!(plain.outline, named.outline, "override changed the map");
    // Whitespace-only override falls back to the seeded name.
    assert_eq!(with_title(Some("  "), Mode::Cave).title, plain.title);
    assert_eq!(
        with_title(Some("Elm Grove"), Mode::Forest).title,
        "Elm Grove"
    );
}

#[test]
fn floor_patterns_follow_tags() {
    use maps_core::decor::PatternElem;
    use maps_core::{GenOptions, generate_with};
    let make = |tags: &str| {
        generate_with(
            11,
            &GenOptions {
                tags: Some(Tags::parse(tags).unwrap()),
                ruins_level: Some(1.0),
                ..GenOptions::default()
            },
        )
    };
    let plain = make("large,chamber,ruins,plain");
    assert!(plain.floor_pattern.is_empty());

    let mosaic = make("large,chamber,ruins,mosaic");
    assert!(!mosaic.floor_pattern.is_empty());
    assert!(mosaic.floor_pattern.iter().all(|e| matches!(e, PatternElem::Poly { .. })));

    let truchet = make("large,chamber,ruins,truchet");
    assert!(!truchet.floor_pattern.is_empty());
    assert!(truchet.floor_pattern.iter().all(|e| matches!(e, PatternElem::Curve { .. })));

    let islamic = make("large,chamber,ruins,islamic");
    assert!(!islamic.floor_pattern.is_empty());
    assert!(islamic.floor_pattern.iter().all(|e| matches!(e, PatternElem::Elbow { .. })));

    // Same seed twice -> identical pattern; the pattern rides the decor
    // stream, so re-rolling decor changes it.
    assert_eq!(truchet.floor_pattern, make("large,chamber,ruins,truchet").floor_pattern);
    let redecor = generate_with(
        11,
        &GenOptions {
            tags: Some(Tags::parse("large,chamber,ruins,truchet").unwrap()),
            ruins_level: Some(1.0),
            decor_seed: Some(999),
            ..GenOptions::default()
        },
    );
    assert_ne!(truchet.floor_pattern, redecor.floor_pattern);

    // A pattern tag without ruins draws nothing.
    let organic = generate_with(
        11,
        &GenOptions {
            tags: Some(Tags::parse("large,chamber,organic,truchet").unwrap()),
            ..GenOptions::default()
        },
    );
    assert!(organic.floor_pattern.is_empty());
}

#[test]
fn grid_styles_render_differently() {
    use maps_core::{GenOptions, GridStyle, generate_with};
    let at = |grid| {
        svg(&generate_with(
            3,
            &GenOptions {
                grid,
                ..GenOptions::default()
            },
        ))
    };
    let hex = at(GridStyle::Hex);
    let square = at(GridStyle::Square);
    let none = at(GridStyle::None);
    assert_ne!(hex, square);
    assert_ne!(square, none);
    // No grid means less markup than either overlay.
    assert!(none.len() < hex.len() && none.len() < square.len());
}

#[test]
fn ruin_decoration_matches_mode() {
    use maps_core::{GenOptions, Mode, generate_with};
    let opts = |mode| GenOptions {
        mode,
        tags: Some(Tags::parse("large,chamber,ruins").unwrap()),
        ruins_level: Some(1.0),
        ..GenOptions::default()
    };
    let cave = generate_with(11, &opts(Mode::Cave));
    assert!(cave.ruins.iter().any(|r| r.is_some()), "no ruins applied");
    assert!(!cave.dots.is_empty(), "cave ruins missing stipple dots");
    assert!(cave.tiles.is_empty(), "cave has masonry tiles");
    let glade = generate_with(11, &opts(Mode::Forest));
    assert!(!glade.tiles.is_empty(), "glade ruins missing masonry tiles");
    assert!(glade.dots.is_empty(), "glade has stipple dots");
    // Organic-only maps produce neither.
    let plain = generate_with(
        11,
        &GenOptions {
            ruins_level: Some(0.0),
            ..GenOptions::default()
        },
    );
    assert!(plain.dots.is_empty() && plain.tiles.is_empty());
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
        // Pin the wet tag: the level fine-tunes an active water state and
        // is ignored by seeds that roll dry.
        let map = generate_with(
            seed,
            &GenOptions {
                tags: Some(Tags::parse("wet").unwrap()),
                water_level: Some(level),
                ..GenOptions::default()
            },
        );
        map.water
    };
    // Net signed area: outer loops and holes carry opposite winding, so the
    // magnitude of the sum is the true flooded area.
    let area = |loops: &Vec<Vec<(f64, f64)>>| {
        loops
            .iter()
            .map(|lp| {
                let mut a = 0.0;
                for j in 0..lp.len() {
                    let (x1, y1) = lp[j];
                    let (x2, y2) = lp[(j + 1) % lp.len()];
                    a += x1 * y2 - x2 * y1;
                }
                a / 2.0
            })
            .sum::<f64>()
            .abs()
    };
    for seed in 0..6 {
        let low = area(&at_level(seed, 0.3));
        let high = area(&at_level(seed, 0.55));
        assert!(
            high >= low,
            "seed {seed}: raising the level shrank the water ({low:.0} -> {high:.0})"
        );
        assert!(at_level(seed, 0.0).is_empty(), "seed {seed}: level 0 not dry");
        // Level 1 submerges the whole floor.
        let full = area(&at_level(seed, 1.0));
        assert!(full > 0.0, "seed {seed}: level 1 not flooded");
        assert!(full >= high, "seed {seed}: level 1 smaller than 0.55");
    }
    // Same seed and level twice -> identical pools (elevation is stable).
    let a: HashSet<String> = at_level(3, 0.4).iter().map(|l| format!("{l:?}")).collect();
    let b: HashSet<String> = at_level(3, 0.4).iter().map(|l| format!("{l:?}")).collect();
    assert_eq!(a, b);
}

#[test]
fn sub_seeds_isolate_streams() {
    use maps_core::{GenOptions, generate_with};
    let base = generate_with(42, &GenOptions::default());

    // A new name seed changes only the title.
    let renamed = generate_with(
        42,
        &GenOptions {
            name_seed: Some(999),
            ..GenOptions::default()
        },
    );
    assert_eq!(base.outline, renamed.outline);
    assert_eq!(base.water, renamed.water);
    assert_eq!(
        base.hatching.len(),
        renamed.hatching.len(),
        "decor changed with the name seed"
    );

    // A new decor seed changes the hatching but not shape or title.
    let redecorated = generate_with(
        42,
        &GenOptions {
            decor_seed: Some(999),
            ..GenOptions::default()
        },
    );
    assert_eq!(base.outline, redecorated.outline);
    assert_eq!(base.title, redecorated.title);
    let strokes = |m: &maps_core::CaveMap| {
        m.hatching
            .iter()
            .flat_map(|f| f.strokes.iter())
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
    };
    assert_ne!(strokes(&base), strokes(&redecorated));

    // A new shape seed changes the map itself.
    let reshaped = generate_with(
        42,
        &GenOptions {
            shape_seed: Some(999),
            ..GenOptions::default()
        },
    );
    assert_ne!(base.outline, reshaped.outline);

    // Supplying the base map's own sub-seeds explicitly replicates it.
    let replica = generate_with(
        7777, // different master seed; every stream pinned
        &GenOptions {
            tags: Some(base.tags.clone()),
            shape_seed: Some(base.shape_seed),
            decor_seed: Some(base.decor_seed),
            name_seed: Some(base.name_seed),
            ..GenOptions::default()
        },
    );
    assert_eq!(base.outline, replica.outline);
    assert_eq!(base.title, replica.title);
    assert_eq!(strokes(&base), strokes(&replica));
}

#[test]
fn ruins_level_controls_geometric_areas() {
    use maps_core::{GenOptions, generate_with};
    let at = |level: Option<f64>| {
        generate_with(
            11,
            &GenOptions {
                tags: Some(Tags::parse("large,chamber").unwrap()),
                ruins_level: level,
                ..GenOptions::default()
            },
        )
    };
    // The organic tag guarantees level 0, same as an explicit override.
    let organic = generate_with(
        11,
        &GenOptions {
            tags: Some(Tags::parse("large,chamber,organic").unwrap()),
            ..GenOptions::default()
        },
    );
    assert_eq!(at(Some(0.0)).outline, organic.outline);
    assert!(organic.ruins.iter().all(|r| r.is_none()));
    // Full ruins reshape the walls.
    assert_ne!(at(Some(1.0)).outline, at(Some(0.0)).outline);
    // Deterministic for the same level.
    assert_eq!(at(Some(1.0)).outline, at(Some(1.0)).outline);
    // The ruins tag alone activates geometry (default level 0.5).
    let tagged = generate_with(
        11,
        &GenOptions {
            tags: Some(Tags::parse("large,chamber,ruins").unwrap()),
            ..GenOptions::default()
        },
    );
    assert_ne!(tagged.outline, at(Some(0.0)).outline);
}

#[test]
fn dungeon_level_splits_geometric_areas() {
    use maps_core::{AreaKind, GenOptions, generate_with};
    let at = |tags: &str, dungeon: Option<f64>| {
        generate_with(
            11,
            &GenOptions {
                tags: Some(Tags::parse(tags).unwrap()),
                ruins_level: Some(1.0),
                dungeon_level: dungeon,
                ..GenOptions::default()
            },
        )
    };

    // Every area is classified, and the geometric (reshaped) set is exactly
    // the non-organic kinds.
    let full = at("large,chamber,ruins,dungeon", Some(1.0));
    assert_eq!(full.area_kind.len(), full.areas.count());
    let geometric = full.ruins.iter().filter(|r| r.is_some()).count();
    assert!(geometric > 0, "no geometric areas to split");
    for (i, k) in full.area_kind.iter().enumerate() {
        assert_eq!(
            full.ruins[i].is_some(),
            *k != AreaKind::Organic,
            "area {i}: geometry and kind disagree"
        );
    }
    // dungeon_level 1.0 promotes every geometric area; none stay Ruin.
    let dungeon = full.area_kind.iter().filter(|k| **k == AreaKind::Dungeon).count();
    assert_eq!(dungeon, geometric, "level 1.0 should promote every geometric area");
    assert!(full.area_kind.iter().all(|k| *k != AreaKind::Ruin));

    // dungeon_level 0 and the `natural` tag leave nothing a dungeon.
    assert!(at("large,chamber,ruins,dungeon", Some(0.0))
        .area_kind
        .iter()
        .all(|k| *k != AreaKind::Dungeon));
    assert!(at("large,chamber,ruins,natural", None)
        .area_kind
        .iter()
        .all(|k| *k != AreaKind::Dungeon));

    // A partial level splits the geometric areas into both kinds.
    let half = at("large,chamber,ruins,dungeon", Some(0.5));
    let half_dungeon = half.area_kind.iter().filter(|k| **k == AreaKind::Dungeon).count();
    assert!(half_dungeon > 0 && half_dungeon < geometric, "0.5 should split, got {half_dungeon}/{geometric}");

    // Clean dungeon walls shed the ruin stipple: an all-dungeon cave has none,
    // while the same map left as ruins does.
    assert!(full.dots.is_empty(), "all-dungeon cave should have no stipple");
    assert!(!at("large,chamber,ruins,natural", None).dots.is_empty(), "ruin cave lost its stipple");

    // The split rides the salt-4 sub-stream and is deterministic.
    assert_eq!(full.area_kind, at("large,chamber,ruins,dungeon", Some(1.0)).area_kind);
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
