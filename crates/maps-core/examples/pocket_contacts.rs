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

mod common;

use maps_core::growth::components;

fn main() {
    let seeds: u64 = common::env("SEEDS", 200);
    for tag_str in common::CONFIGS {
        let (mut contacts, mut conns, mut widths) = (0usize, 0usize, 0usize);
        let (mut split_maps, mut split_areas) = (0usize, 0usize);
        let mut contact_seeds: Vec<u64> = Vec::new();
        for seed in 1..=seeds {
            let m = common::generate(seed, tag_str);
            // Claimed corridors only: an unclaimed passage is a free gap with no wall, so it
            // has no barrier to lose.
            let runs = common::run_sets(m.topology.connections.iter().filter(|c| c.claimed));
            let mut hit = false;
            for (i, run) in runs.iter().enumerate() {
                conns += 1;
                widths += run.len();
                let touch = runs
                    .iter()
                    .enumerate()
                    .any(|(j, other)| j != i && common::runs_touch(run, other));
                if touch {
                    contacts += 1;
                    hit = true;
                }
            }
            if hit {
                contact_seeds.push(seed);
            }
            // Areas the connection graph plus the fusion seams do not reach.
            let areas = &m.areas;
            let n = areas.count();
            let seams = (0..n).flat_map(|i| {
                areas.floor_cells(i).flat_map(move |c| {
                    c.neighbors().into_iter().filter_map(move |nb| {
                        areas.owner_of(nb).filter(|&o| o != i).map(|o| (i, o))
                    })
                })
            });
            let comp = components(
                n,
                seams.chain(m.topology.connections.iter().map(|c| (c.a, c.b))),
            );
            let cut = (0..n).filter(|&i| comp[i] != comp[0]).count();
            if cut > 0 {
                split_maps += 1;
                split_areas += cut;
            }
        }
        println!(
            "{tag_str:>46}  contacts {contacts:4}  conns {conns:5}  mean_width {:.2}  \
             contact_maps {}  split_maps {split_maps}  split_areas {split_areas}",
            widths as f64 / conns as f64,
            contact_seeds.len()
        );
        print!("    seeds:");
        for s in contact_seeds.iter().take(24) {
            print!(" {s}");
        }
        println!();
    }
}
