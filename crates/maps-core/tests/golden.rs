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

/// (seed, tags or "" for seed-rolled, mode, grid, water_level, ruins_level)
type Case = (u64, &'static str, &'static str, &'static str, Option<f64>, Option<f64>);

const CASES: &[Case] = &[
    (1, "small,chamber,organic,plain", "cave", "hex", None, None),
    (2, "medium,coral,wet,organic,plain", "cave", "hex", None, None),
    (3, "large,hub,wet,organic,plain", "cave", "hex", Some(0.4), None),
    (11, "large,burrow,tree,junction,dry,ruins,truchet", "cave", "none", None, Some(0.9)),
    (7, "medium,connected,wet,organic,plain", "forest", "hex", Some(0.3), None),
    (19, "large,chamber,connected,ruins,mosaic", "forest", "hex", Some(0.3), Some(0.75)),
    (13, "large,cavities,sealed,organic,islamic", "cave", "square", Some(1.0), None),
    (17, "large,chaotic,entrance,wet,organic,plain", "cave", "square", Some(0.05), None),
    (42, "", "cave", "hex", None, None),
    (99, "", "forest", "none", None, None),
    (19, "large,chamber,connected,wet,ruins,islamic", "forest", "hex", Some(0.3), Some(0.85)),
    (9521512733245147772, "medium,hub,coral,tree,junction,dry,ruins,plain", "cave", "hex", None, Some(0.5)),
];

/// Expected (svg, debug_svg) FNV-1a hashes, one pair per case above.
const GOLDEN: &[(u64, u64)] = &[
    (0x257a164984650d35, 0xfafd508cfddfa386),
    (0xbf41a95b4d1b4fbc, 0xb19fd022c1f6238c),
    (0x95ae8a344a79f6fd, 0x72cbc40a38d25efb),
    (0xbb8bd1b845495051, 0x2a08d7e6a6266fbe),
    (0x31054d8c9fe0a529, 0x9938523f6cfb0d62),
    (0xb33e90ab0e2e7d93, 0x0b7e47e69805b909),
    (0x8176a7a1f815b45e, 0x09098b237fdf5850),
    (0x2924edf7f1ee13f0, 0xde32b742f2300afb),
    (0x01d520339f2c39a9, 0x5fe1d02b7cf03562),
    (0xe0aa26f87a0e6d58, 0x270856ff05ab68eb),
    (0xc310519bcd0639ab, 0xc56ae08aa139a57a),
    (0x06e1c56cd22c45f3, 0x61abc1d2ca40a380),
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
    let (seed, tags, mode, grid, water, ruins) = *case;
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
