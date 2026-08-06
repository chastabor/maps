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

use maps_core::grid::Hex;
use maps_core::tags::Tags;
use maps_core::{AreaKind, GenOptions, generate_with};
use std::collections::HashSet;

fn area_hash(cells: &[Hex]) -> String {
    let mut v: Vec<(i32, i32)> = cells.iter().map(|c| (c.q, c.r)).collect();
    v.sort_unstable();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (q, r) in v {
        for b in q.to_le_bytes().iter().chain(r.to_le_bytes().iter()) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = [0u8; 4];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(h % 36) as usize];
        h /= 36;
    }
    String::from_utf8(out.to_vec()).unwrap()
}

fn main() {
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(82);
    let tag_str =
        std::env::var("TAGS").unwrap_or_else(|_| "large,ruins,dungeon,separate".to_string());
    let m = generate_with(
        seed,
        &GenOptions {
            tags: Some(Tags::parse(&tag_str).unwrap()),
            ..GenOptions::default()
        },
    );
    let label = |i: usize| {
        let cells: Vec<Hex> = m.areas.floor_cells(i).collect();
        let k = match m.areas.kind(i) {
            AreaKind::Dungeon => "D",
            AreaKind::Ruin => "R",
            AreaKind::Organic => "O",
        };
        format!("{i}{k}/{}", area_hash(&cells))
    };
    let runs: Vec<HashSet<Hex>> = m
        .topology
        .connections
        .iter()
        .map(|c| c.along[..c.apron_from].iter().copied().collect())
        .collect();
    println!("seed {seed}  tags {tag_str}  areas {}", m.areas.count());
    for (i, c) in m.topology.connections.iter().enumerate() {
        let touch: Vec<usize> = runs
            .iter()
            .enumerate()
            .filter(|&(j, other)| {
                j != i
                    && runs[i]
                        .iter()
                        .any(|h| h.neighbors().iter().any(|n| other.contains(n)))
            })
            .map(|(j, _)| j)
            .collect();
        println!(
            "  {i:2} {:>10} <-> {:<10} run {:?} apron {:?}{}",
            label(c.a),
            label(c.b),
            &c.along[..c.apron_from]
                .iter()
                .map(|h| (h.q, h.r))
                .collect::<Vec<_>>(),
            &c.along[c.apron_from..]
                .iter()
                .map(|h| (h.q, h.r))
                .collect::<Vec<_>>(),
            if touch.is_empty() {
                String::new()
            } else {
                format!("  TOUCHES {touch:?}")
            }
        );
    }
}
