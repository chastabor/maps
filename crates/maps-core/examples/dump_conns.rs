//! Every connection of one map: the areas it joins, its run and apron cells, and which other
//! connections' runs it touches.
//!
//! The per-map counterpart to `pocket_contacts`' range statistics, and where to start once that
//! reports a number worth explaining — the area labels (`5D/dngl`) are the same ones the finished
//! render and `growth_view` print, so a row here names a room you can point at. `cell_map` then
//! draws the cells a row lists.
//!
//! ```text
//! SEED=82 TAGS=large,ruins,dungeon,separate cargo run -p maps-core --release --example dump_conns
//! ```

mod common;

use maps_core::grid::Hex;
use maps_core::render::area_hash;

fn main() {
    let seed: u64 = common::env("SEED", 82);
    let tag_str = common::tags_env();
    let m = common::generate(seed, &tag_str);
    let labels: Vec<String> = (0..m.areas.count())
        .map(|i| {
            let cells: Vec<Hex> = m.areas.floor_cells(i).collect();
            format!("{i}{}/{}", m.areas.kind(i).letter(), area_hash(&cells))
        })
        .collect();
    let runs = common::run_sets(&m.topology.connections);
    println!("seed {seed}  tags {tag_str}  areas {}", m.areas.count());
    for (i, c) in m.topology.connections.iter().enumerate() {
        let touch: Vec<usize> = runs
            .iter()
            .enumerate()
            .filter(|&(j, other)| j != i && common::runs_touch(&runs[i], other))
            .map(|(j, _)| j)
            .collect();
        println!(
            "  {i:2} {:>10} <-> {:<10} run {:?} apron {:?}{}",
            labels[c.a],
            labels[c.b],
            c.run().iter().map(|h| (h.q, h.r)).collect::<Vec<_>>(),
            c.apron().iter().map(|h| (h.q, h.r)).collect::<Vec<_>>(),
            if touch.is_empty() {
                String::new()
            } else {
                format!("  TOUCHES {touch:?}")
            }
        );
    }
}
