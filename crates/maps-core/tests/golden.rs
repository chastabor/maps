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
    (0x1041f0e4fd15575c, 0xfafd508cfddfa386),
    (0x57e1fa431d0b86d6, 0xb19fd022c1f6238c),
    (0xc74c435895404486, 0x72cbc40a38d25efb),
    (0x8ba51059d77dd7b7, 0x2a08d7e6a6266fbe),
    (0x11522166744a091c, 0x9938523f6cfb0d62),
    (0x8b9fbd273a37ebdd, 0x0b7e47e69805b909),
    (0x714b6500ac907bf5, 0x09098b237fdf5850),
    (0xbaee194cfc14ff8c, 0xde32b742f2300afb),
    (0xb65eac14ed54e489, 0xa662fdb70b19b5ef),
    (0xf822a1e78b985eee, 0xb33bc133643db095),
    (0x6732b76d34127b40, 0xc56ae08aa139a57a),
    (0xa8762e6d3fdb5817, 0x61abc1d2ca40a380),
    (0xbce0f4accc2d902c, 0x80b01d50a41eb59e),
    (0xceb09916edc7f031, 0x0f8ac5e0d43264ce),
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
