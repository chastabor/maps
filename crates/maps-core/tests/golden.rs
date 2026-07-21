//! Byte-identity harness for performance refactors: hashes of the exact SVG
//! output (finished render + debug render) for a case matrix covering every
//! generation path. Any refactor that changes a single byte of output for
//! any case fails here.
//!
//! To (re)generate the table after an *intentional* output change:
//! cargo test -p maps-core --test golden print_golden -- --nocapture --ignored

use maps_core::render::{debug_svg, svg};
use maps_core::tags::Tags;
use maps_core::{GenOptions, GridStyle, Mode, generate_with};

/// (seed, tags or "" for seed-rolled, mode, grid, water_level, ruins_level,
/// dungeon_level)
type Case = (
    u64,
    &'static str,
    &'static str,
    &'static str,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

const CASES: &[Case] = &[
    (1, "small,chamber,organic,plain", "cave", "hex", None, None, None),
    (2, "medium,coral,wet,organic,plain", "cave", "hex", None, None, None),
    (3, "large,hub,wet,organic,plain", "cave", "hex", Some(0.4), None, None),
    (11, "large,burrow,tree,junction,dry,ruins,truchet", "cave", "none", None, Some(0.9), None),
    (7, "medium,connected,wet,organic,plain", "forest", "hex", Some(0.3), None, None),
    (19, "large,chamber,connected,ruins,mosaic", "forest", "hex", Some(0.3), Some(0.75), None),
    (13, "large,cavities,sealed,organic,islamic", "cave", "square", Some(1.0), None, None),
    (17, "large,chaotic,entrance,wet,organic,plain", "cave", "square", Some(0.05), None, None),
    (42, "", "cave", "hex", None, None, None),
    (99, "", "forest", "none", None, None, None),
    (19, "large,chamber,connected,wet,ruins,islamic", "forest", "hex", Some(0.3), Some(0.85), None),
    (9521512733245147772, "medium,hub,coral,tree,junction,dry,ruins,plain", "cave", "hex", None, Some(0.5), None),
    // Dungeon path: geometric areas split into clean, doorless-decor rooms.
    // Cave exercises the hatching/stipple skip; forest the canopy/masonry skip.
    (11, "large,burrow,tree,junction,dry,ruins,dungeon,plain", "cave", "none", None, Some(1.0), Some(0.5)),
    (19, "large,chamber,connected,ruins,dungeon,mosaic", "forest", "hex", Some(0.3), Some(0.9), Some(0.6)),
];

/// Expected (svg, debug_svg) FNV-1a hashes, one pair per case above.
const GOLDEN: &[(u64, u64)] = &[
    (0x9feafdd5e2caea36, 0x001aafd5eb5a7ea0),
    (0x3e9d82dc04477173, 0x14c826b69ab9805f),
    (0x813d3624ba4c358b, 0x390a31e1c1a4aa05),
    (0x58575469ddd3b9b0, 0x3016b8431c8979af),
    (0xd828d101e38ac881, 0x07df8211b7c87b02),
    (0x04d5074491784394, 0x7f5f1d3fcba533dc),
    (0xb347d8a0089df946, 0x2eb2b20989f053db),
    (0x3948215648b7a397, 0x1c53b4bcf1b97a72),
    (0x0e816360aca3dfad, 0x5dfefa40f7d15a1c),
    (0x5e815f5295d09e72, 0x3509ce8ca9218ee0),
    (0xa1dc351eb55d609c, 0x08901af2552b82a9),
    (0x5783e152d9f8dd72, 0x9d8f63f6766e8393),
    (0xa792bcc6bec80efa, 0x0106ddbf93c548c9),
    (0x6d73fd0b7e37ac2e, 0xbdc2f20027459728),
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
    let (seed, tags, mode, grid, water, ruins, dungeon) = *case;
    let map = generate_with(
        seed,
        &GenOptions {
            mode: if mode == "forest" { Mode::Forest } else { Mode::Cave },
            grid: match grid {
                "square" => GridStyle::Square,
                "none" => GridStyle::None,
                _ => GridStyle::Hex,
            },
            tags: (!tags.is_empty()).then(|| Tags::parse(tags).unwrap()),
            water_level: water,
            ruins_level: ruins,
            dungeon_level: dungeon,
            ..GenOptions::default()
        },
    );
    (fnv1a(svg(&map).as_bytes()), fnv1a(debug_svg(&map).as_bytes()))
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
