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
    (0xfd472b27d048ca57, 0xc5112a7194d6155e),
    (0xd82dc9a836c25577, 0x8c7855f7fd634bf1),
    (0x4fcc0d0aebbae919, 0x80984b85ad716e5d),
    (0x0d9c11b3f306081d, 0xf29a20848d3d67e2),
    (0x7ba133d11b70552d, 0xdf3de61c835df099),
    (0x972e9f1e937f6ab8, 0x7a8e8e857f019dad),
    (0x7481d03715f58551, 0x273409efba55426f),
    (0x8c24fc31f12202cb, 0x7107bfc9c3e59191),
    (0xc4bb2f3bc06bcb5a, 0xf316093609e303d9),
    (0xbb687209d37af843, 0x17965f98deb4217b),
    (0x501bcfa842bee554, 0xe4095a186fed5e5c),
    (0x76ad2be9faa5b85d, 0x05eb5ffc1c61bf63),
    (0x57e9e2ceca4d7081, 0x571c383f8a2af7d4),
    (0x258dd9492d031d8d, 0xa33a264fd9ae1529),
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
