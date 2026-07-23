//! Throwaway timing probe (not a real benchmark harness). Run with:
//! cargo test --release -p maps-core --test bench -- --nocapture --ignored

use maps_core::render::svg;
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};
use std::time::Instant;

#[test]
#[ignore]
fn stages() {
    use maps_core::grid::HexGrid;
    use maps_core::growth::{grid_radius, grow_areas, resolve};
    use maps_core::outline::{OutlineParams, build_outline};
    use maps_core::{render, ruins, topology, water};
    use rand::SeedableRng;
    use rand_pcg::Pcg64;

    let tags = Tags::parse("large,burrow,wet,ruins,truchet").unwrap();
    let n = 60;
    let mut acc = [0.0f64; 8];
    for seed in 0..n {
        let oparams = OutlineParams::default();
        let mut rng = Pcg64::seed_from_u64(1000 + seed);
        let t = Instant::now();
        let params = resolve(&tags, &mut rng);
        let grid = HexGrid::hexagon(grid_radius(&params));
        // Time the ruin path: classify every area geometric.
        let slot_kinds = vec![maps_core::AreaKind::Ruin; params.sizes.len()];
        let slot_fusible = vec![false; params.sizes.len()];
        let mut areas = grow_areas(&grid, &mut rng, &params, &slot_kinds, &slot_fusible, oparams.hex_size);
        acc[0] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let topo = topology::build(&grid, &mut areas, &tags, oparams.hex_size, &mut rng);
        acc[1] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        ruins::build(&mut areas, &topo, oparams.hex_size, &mut rng);
        acc[2] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let ruin_map = ruins::ruin_cell_map(&areas, oparams.hex_size);
        let (outline, _walls) =
            build_outline(&areas, &topo, &ruin_map, &std::collections::HashMap::new(), &[], &oparams, &mut rng);
        acc[3] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let _w = water::build_water(&areas, &topo, &oparams, &tags, Some(0.4), &mut rng);
        acc[4] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let ruin_cells: std::collections::HashSet<_> = ruin_map.keys().copied().collect();
        let _fans = maps_core::decor::hatching(&outline, &ruin_cells, &[], 12.0, &mut rng);
        acc[5] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let map = generate_with(seed, &GenOptions {
            tags: Some(tags.clone()),
            ruins_level: Some(0.9),
            water_level: Some(0.4),
            shape_seed: Some(1000 + seed),
            ..GenOptions::default()
        });
        acc[6] += t.elapsed().as_secs_f64();

        let t = Instant::now();
        std::hint::black_box(render::svg(&map));
        acc[7] += t.elapsed().as_secs_f64();
    }
    let ms = |i: usize| acc[i] * 1000.0 / n as f64;
    println!("growth   {:6.2} ms", ms(0));
    println!("topology {:6.2} ms", ms(1));
    println!("ruins    {:6.2} ms", ms(2));
    println!("outline  {:6.2} ms", ms(3));
    println!("water    {:6.2} ms", ms(4));
    println!("decor    {:6.2} ms", ms(5));
    println!("full gen {:6.2} ms", ms(6));
    println!("svg      {:6.2} ms", ms(7));
}

#[test]
#[ignore]
fn timing() {
    for (label, tags, ruins) in [
        ("small organic", "small,chamber,organic,plain", None),
        ("large organic", "large,burrow,wet,organic,plain", None),
        ("large ruins+truchet", "large,burrow,wet,ruins,truchet", Some(0.9)),
        ("large hub coral", "large,hub,coral,wet,organic,plain", None),
    ] {
        let opts = |seed_offset: u64| GenOptions {
            tags: Some(Tags::parse(tags).unwrap()),
            ruins_level: ruins,
            shape_seed: Some(1000 + seed_offset),
            ..GenOptions::default()
        };
        // Warm up.
        let map = generate_with(1, &opts(0));
        let cells: usize = map.areas.cells.iter().map(|a| a.len()).sum();
        let n = 40;
        let t0 = Instant::now();
        for i in 0..n {
            std::hint::black_box(generate_with(i, &opts(i)));
        }
        let gen_ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t1 = Instant::now();
        for _ in 0..n {
            std::hint::black_box(svg(&map));
        }
        let svg_ms = t1.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("{label:22} ~{cells:4} cells  generate {gen_ms:7.2} ms  svg {svg_ms:6.2} ms");
    }
}
