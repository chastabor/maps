//! Corridor-crowding statistics over a seed range: how many claimed corridors touch another
//! corridor's floor, how wide they come out, and how many areas end up unreachable.
//!
//! The one-cell rock barrier between passages is a *statistical* property — any single map can look
//! fine while a rule quietly closes gaps across the range — so this is the measurement the
//! shared-pocket rule in `topology` was built against (`plans/tile-corridor-render.md`).
//! `contacts` counts claimed corridors with a neighbouring corridor's floor beside them; `conns`
//! how many claimed corridors were examined; `split_maps`/`split_areas` how much of the map no
//! connection and no fusion seam reaches.
//!
//! ```text
//! SEEDS=200 cargo run -p maps-core --release --example pocket_contacts
//! ```

use maps_core::grid::Hex;
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};
use std::collections::HashSet;

const CONFIGS: [&str; 3] = [
    "large,ruins,dungeon,separate",
    "large,ruins,dungeon,fused",
    "large,chamber,connected,ruins,dungeon,truchet",
];

fn main() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    for tag_str in CONFIGS {
        let tags = Tags::parse(tag_str).unwrap();
        let (mut contacts, mut conns, mut widths) = (0usize, 0usize, 0usize);
        let (mut split_maps, mut split_areas) = (0usize, 0usize);
        let mut worst: Vec<u64> = Vec::new();
        for seed in 1..=seeds {
            let m = generate_with(
                seed,
                &GenOptions {
                    tags: Some(tags.clone()),
                    ..GenOptions::default()
                },
            );
            let joined: Vec<bool> = m
                .topology
                .connections
                .iter()
                .map(|c| c.along[..c.apron_from].iter().any(|h| m.areas.is_join(*h)))
                .collect();
            let runs: Vec<HashSet<Hex>> = m
                .topology
                .connections
                .iter()
                .map(|c| c.along[..c.apron_from].iter().copied().collect())
                .collect();
            let mut hit = false;
            for (i, run) in runs.iter().enumerate() {
                if !joined[i] {
                    continue;
                }
                conns += 1;
                widths += run.len();
                let touch = runs.iter().enumerate().any(|(j, other)| {
                    j != i
                        && joined[j]
                        && run
                            .iter()
                            .any(|c| c.neighbors().iter().any(|n| other.contains(n)))
                });
                if touch {
                    contacts += 1;
                    hit = true;
                }
            }
            if hit {
                worst.push(seed);
            }
            // Areas the connection graph plus the fusion seams do not reach.
            let n = m.areas.count();
            let mut parent: Vec<usize> = (0..n).collect();
            fn find(p: &mut Vec<usize>, x: usize) -> usize {
                if p[x] != x {
                    let r = find(p, p[x]);
                    p[x] = r;
                }
                p[x]
            }
            for i in 0..n {
                for c in m.areas.floor_cells(i) {
                    for nb in c.neighbors() {
                        if let Some(o) = m.areas.owner_of(nb).filter(|&o| o != i) {
                            let (ri, ro) = (find(&mut parent, i), find(&mut parent, o));
                            parent[ri] = ro;
                        }
                    }
                }
            }
            for c in &m.topology.connections {
                let (ra, rb) = (find(&mut parent, c.a), find(&mut parent, c.b));
                parent[ra] = rb;
            }
            let root0 = find(&mut parent, 0);
            let cut = (0..n).filter(|&i| find(&mut parent, i) != root0).count();
            if cut > 0 {
                split_maps += 1;
                split_areas += cut;
            }
        }
        println!(
            "{tag_str:>46}  contacts {contacts:4}  conns {conns:5}  mean_width {:.2}  \
             seeds {}  split_maps {split_maps}  split_areas {split_areas}",
            widths as f64 / conns as f64,
            worst.len()
        );
        print!("    seeds:");
        for s in worst.iter().take(24) {
            print!(" {s}");
        }
        println!();
    }
}
