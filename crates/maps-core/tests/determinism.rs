//! Generation depends on the seed and nothing else.
//!
//! Worth asserting rather than assuming, because the plans use content digests as their
//! containment guard — "this configuration's digest did not move, so the change was confined
//! to the others". A digest that will not hold still makes that argument worthless, and it
//! fails quietly: the metrics beside it (`si`, connector counts) look plausible either way.
//!
//! The bug this was written for: `keep_largest_component` picked the biggest connected
//! component with `HashMap::iter().max_by_key(..)`, which keeps the last maximum it sees. Two
//! components of equal size were therefore separated by hash iteration order, randomised per
//! process, and a different set of areas survived from run to run — different doors, exits and
//! outline. It needed a size tie, so it struck about one seed in a hundred.
//!
//! `SEEDS` widens the sweep (default 24).

use maps_core::render::{debug_svg, svg};
use maps_core::tags::Tags;
use maps_core::{GenOptions, generate_with};

const CONFIGS: [&str; 5] = [
    "large,organic,separate",
    "medium,coral,wet,organic,mosaic",
    "large,ruins,dungeon,separate",
    "large,ruins,dungeon,fused",
    "large,chamber,connected,ruins,dungeon,truchet",
];

/// Levels are forced high: the tag defaults leave geometry sparse, and every mechanism that
/// has drifted so far lives in the geometric path.
fn opts(tags: &str) -> GenOptions {
    GenOptions {
        tags: Some(Tags::parse(tags).unwrap()),
        ruins_level: Some(1.0),
        dungeon_level: Some(1.0),
        fuse_level: Some(1.0),
        ..GenOptions::default()
    }
}

/// Both renders, so a divergence anywhere in the pipeline shows up.
fn bytes(seed: u64, tags: &str) -> (String, String) {
    let m = generate_with(seed, &opts(tags));
    (svg(&m), debug_svg(&m))
}

/// The witness for the `keep_largest_component` tie. Kept as its own case because it is
/// cheap and exact — a broad sweep needs ~100 seeds to stumble on a size tie, and a fast
/// default test that only sometimes covers the bug is worse than one that always does.
#[test]
fn the_component_tie_seed_is_stable() {
    let tags = "large,ruins,dungeon,separate";
    let (a1, b1) = bytes(23, tags);
    let (a2, b2) = bytes(23, tags);
    assert_eq!(
        a1.len(),
        a2.len(),
        "seed 23 [{tags}]: svg length differs between runs"
    );
    assert!(
        a1 == a2 && b1 == b2,
        "seed 23 [{tags}]: output differs between runs"
    );
}

#[test]
fn repeated_generation_is_byte_identical() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    for tags in CONFIGS {
        for seed in 1..=seeds {
            let (a1, b1) = bytes(seed, tags);
            let (a2, b2) = bytes(seed, tags);
            assert!(
                a1 == a2 && b1 == b2,
                "seed {seed} [{tags}]: two generations of one seed differ — something outside \
                 the seed is being read (hash iteration order is the usual cause)"
            );
        }
    }
}
