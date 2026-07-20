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
    (0x1041f0e4fd15575c, 0x33ab03ac36723785),
    (0x57e1fa431d0b86d6, 0xe014d36ab27070e3),
    (0xc74c435895404486, 0x56901131758157f2),
    (0x9fc6ef0dbdc3ea03, 0x311f9421c2126725),
    (0x11522166744a091c, 0x6ea2e6c138f5ea7b),
    (0xd9af56561cc815f1, 0xb5ac0e35d5943f93),
    (0x714b6500ac907bf5, 0xe47fe0de14ae26eb),
    (0xbaee194cfc14ff8c, 0xcc9a2f51fba97a42),
    (0xbd31e89524273152, 0x751390cce51b96fe),
    (0xf822a1e78b985eee, 0x572fffa08eb5fe7a),
    (0x21a4bc045516ce21, 0xaf2dd98b5abbc70d),
    (0x43b817be68767858, 0x7a005dbd5d630b4e),
    (0xae9f1a71d0e37742, 0xbf70339fc3ca3501),
    (0x9b3d1d1d094a9eed, 0x07d17817228d7c07),
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
