//! Byte-identity harness for performance refactors: hashes of the exact SVG
//! output (finished render + debug render) for a case matrix covering every
//! generation path. Any refactor that changes a single byte of output for
//! any case fails here.
//!
//! **Fusion is covered explicitly.** For a long time it was not: no case tag named `fused`, both
//! seed-rolled cases happen to roll `fuse: Separate`, and `Case` had no `fuse_level` field at
//! all — so the whole connector/seam/clipping path was invisible here. A change that rewrote
//! every fused map passed this file with all 14 hashes unmoved, which is exactly the false
//! assurance a byte-identity harness must not give.
//!
//! To (re)generate the table after an *intentional* output change:
//! cargo test -p maps-core --test golden print_golden -- --nocapture --ignored

use maps_core::render::{debug_svg, svg};
use maps_core::tags::Tags;
use maps_core::{GenOptions, GridStyle, Mode, generate_with};

/// (seed, tags or "" for seed-rolled, mode, grid, water_level, ruins_level,
/// dungeon_level, fuse_level)
type Case = (
    u64,
    &'static str,
    &'static str,
    &'static str,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

const CASES: &[Case] = &[
    (
        1,
        "small,chamber,organic,plain",
        "cave",
        "hex",
        None,
        None,
        None,
        None,
    ),
    (
        2,
        "medium,coral,wet,organic,plain",
        "cave",
        "hex",
        None,
        None,
        None,
        None,
    ),
    (
        3,
        "large,hub,wet,organic,plain",
        "cave",
        "hex",
        Some(0.4),
        None,
        None,
        None,
    ),
    (
        11,
        "large,burrow,tree,junction,dry,ruins,truchet",
        "cave",
        "none",
        None,
        Some(0.9),
        None,
        None,
    ),
    (
        7,
        "medium,connected,wet,organic,plain",
        "forest",
        "hex",
        Some(0.3),
        None,
        None,
        None,
    ),
    (
        19,
        "large,chamber,connected,ruins,mosaic",
        "forest",
        "hex",
        Some(0.3),
        Some(0.75),
        None,
        None,
    ),
    (
        13,
        "large,cavities,sealed,organic,islamic",
        "cave",
        "square",
        Some(1.0),
        None,
        None,
        None,
    ),
    (
        17,
        "large,chaotic,entrance,wet,organic,plain",
        "cave",
        "square",
        Some(0.05),
        None,
        None,
        None,
    ),
    (42, "", "cave", "hex", None, None, None, None),
    (99, "", "forest", "none", None, None, None, None),
    (
        19,
        "large,chamber,connected,wet,ruins,islamic",
        "forest",
        "hex",
        Some(0.3),
        Some(0.85),
        None,
        None,
    ),
    (
        9521512733245147772,
        "medium,hub,coral,tree,junction,dry,ruins,plain",
        "cave",
        "hex",
        None,
        Some(0.5),
        None,
        None,
    ),
    // Dungeon path: geometric areas split into clean, doorless-decor rooms.
    // Cave exercises the hatching/stipple skip; forest the canopy/masonry skip.
    (
        11,
        "large,burrow,tree,junction,dry,ruins,dungeon,plain",
        "cave",
        "none",
        None,
        Some(1.0),
        Some(0.5),
        None,
    ),
    (
        19,
        "large,chamber,connected,ruins,dungeon,mosaic",
        "forest",
        "hex",
        Some(0.3),
        Some(0.9),
        Some(0.6),
        None,
    ),
    // Fusion: the connector/seam/clip path, which nothing above reaches.
    (
        7,
        "large,ruins,dungeon,fused",
        "cave",
        "hex",
        None,
        Some(1.0),
        Some(1.0),
        Some(1.0),
    ),
    (
        23,
        "large,chamber,connected,ruins,dungeon,fused,truchet",
        "cave",
        "hex",
        None,
        Some(1.0),
        Some(1.0),
        Some(1.0),
    ),
    (
        99,
        "large,ruins,dungeon,fused",
        "forest",
        "square",
        Some(0.3),
        Some(0.8),
        Some(0.6),
        Some(0.71),
    ),
];

/// Expected (svg, debug_svg) FNV-1a hashes, one pair per case above.
const GOLDEN: &[(u64, u64)] = &[
    (0xe9cae0ab5a9d41ce, 0x66d0b93eca6ca738),
    (0x53b782df7158f8a2, 0xc58502feee58489f),
    (0x38cee2587f78ab49, 0x18cfc636155cd11e),
    (0xb3d2e181199bc45f, 0x3a9d3f48c49f7c20),
    (0xfb993168ebe81131, 0xc77b7ff7fd6b3e07),
    (0xcfb62c46d7863ddc, 0xe72e8fd5da10da62),
    (0xba8ae34769ce3500, 0xca5a64b0d84eca45),
    (0x66d8d9161e35dfd3, 0xc1d83fc0da58eddc),
    (0xc68879072672e656, 0x637b147b9f303028),
    (0x5ca4b8a0bbb62435, 0x8455dbb0b2e060ee),
    (0x72e82d5c55603114, 0x3dab226b58621735),
    (0xff024c77df165aab, 0x5c4ec946af97249e),
    (0x082628e66d38ff59, 0x437d6c32fac02455),
    (0xbb16c8f1c501da0b, 0xd6f7e8973e9a9a00),
    (0x19754b964033e605, 0x2d5df9b87cc58fa5),
    (0x577d1d9e009c28d0, 0x3145e752e87a7053),
    (0x27d8b744a4afba23, 0xd4da6700cec7a3c4),
];

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn run(case: &Case) -> (u64, u64) {
    let (seed, tags, mode, grid, water, ruins, dungeon, fuse) = *case;
    let map = generate_with(
        seed,
        &GenOptions {
            mode: if mode == "forest" {
                Mode::Forest
            } else {
                Mode::Cave
            },
            grid: match grid {
                "square" => GridStyle::Square,
                "none" => GridStyle::None,
                _ => GridStyle::Hex,
            },
            tags: (!tags.is_empty()).then(|| Tags::parse(tags).unwrap()),
            water_level: water,
            ruins_level: ruins,
            dungeon_level: dungeon,
            fuse_level: fuse,
            ..GenOptions::default()
        },
    );
    (
        fnv1a(svg(&map).as_bytes()),
        fnv1a(debug_svg(&map).as_bytes()),
    )
}

#[test]
fn outputs_match_golden_hashes() {
    assert_eq!(CASES.len(), GOLDEN.len());
    for (i, case) in CASES.iter().enumerate() {
        let got = run(case);
        assert_eq!(
            got, GOLDEN[i],
            "case {i} (seed {}, tags '{}') output changed",
            case.0, case.1
        );
    }
}

#[test]
#[ignore]
fn print_golden() {
    for case in CASES {
        let (s, d) = run(case);
        println!("    (0x{s:016x}, 0x{d:016x}),");
    }
}
