//! Helpers shared by the debug examples (`cell_map`, `dump_conns`, `pocket_contacts`).
//!
//! Each example compiles this module independently (`mod common;`) and uses a subset of it,
//! so the unused remainder is expected — hence the file-wide allow. Living in a subdirectory
//! keeps cargo from treating it as an example of its own.
#![allow(dead_code)]

use maps_core::grid::Hex;
use maps_core::tags::Tags;
use maps_core::topology::Connection;
use maps_core::{CaveMap, GenOptions, generate_with};
use std::collections::HashSet;

/// An env-var knob with a default: `env("SEEDS", 200u64)`.
pub fn env<T: std::str::FromStr>(k: &str, d: T) -> T {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}

/// The `TAGS` knob, defaulting to the configuration most of the corridor work is measured on.
pub fn tags_env() -> String {
    std::env::var("TAGS").unwrap_or_else(|_| "large,ruins,dungeon,separate".to_string())
}

/// One map from a seed and a tag string — the invocation every example shares.
pub fn generate(seed: u64, tags: &str) -> CaveMap {
    generate_with(
        seed,
        &GenOptions {
            tags: Tags::parse(tags).ok(),
            ..GenOptions::default()
        },
    )
}

/// Each connection's free-run cells as a set — the shape both touch scans index into.
pub fn run_sets<'a>(conns: impl IntoIterator<Item = &'a Connection>) -> Vec<HashSet<Hex>> {
    conns
        .into_iter()
        .map(|c| c.run().iter().copied().collect())
        .collect()
}

/// Whether one run touches another: any cell of `run` hex-adjacent to a cell of `other`.
pub fn runs_touch(run: &HashSet<Hex>, other: &HashSet<Hex>) -> bool {
    run.iter()
        .any(|c| c.neighbors().iter().any(|n| other.contains(n)))
}
