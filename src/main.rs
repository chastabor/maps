use maps::config::Config;
use maps::core::render::{debug_svg, svg};
use maps::core::tags::Tags;
use maps::core::{GenOptions, GridStyle, Mode, generate_with};
use std::path::Path;
use std::process::exit;

const USAGE: &str = "\
Usage: maps [OPTIONS] [CONFIG]

Generates a cave map SVG. Options are read from the TOML config file
(default: maps.toml if present), with command-line flags taking precedence.

Options:
  -c, --config <FILE>  config file path (same as the positional argument)
  -m, --mode <MODE>    map type: cave (default) or forest
  -g, --grid <STYLE>   grid overlay: hex (default), square or none
  -s, --seed <N>       master RNG seed (default: derived from the clock)
      --shape-seed <N> re-roll/pin just the map shape (outline, water, stones)
      --decor-seed <N> re-roll/pin just the hatch fans / tree canopies
      --name-seed <N>  re-roll/pin just the title
  -t, --tags <LIST>    comma-separated tags, e.g. large,hub,coral
                       size:   small|medium|large
                       layout: hub|chamber|burrow
                       shape:  cavities|coral|chaotic
                       links:  tree|connected
                       exits:  sealed|entrance|passage|junction
                       water:  wet|dry
                       ruins:  ruins|organic
  -w, --water <LEVEL>  water level 0.0..=1.0 (0 = dry, 1 = fully submerged)
  -r, --ruins <LEVEL>  ruins level 0.0..=1.0: fraction of areas that become
                       geometric (rectangles/circles) instead of organic
  -o, --out <FILE>     output SVG path (default: cave.svg)
  -d, --debug          render raw hex cells instead of the finished map
  -h, --help           show this help";

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(1);
}

fn main() {
    let mut config_path: Option<String> = None;
    let mut mode: Option<Mode> = None;
    let mut grid: Option<GridStyle> = None;
    let mut seed: Option<u64> = None;
    let mut tags: Option<Tags> = None;
    let mut out: Option<String> = None;
    let mut debug: Option<bool> = None;
    let mut water_level: Option<f64> = None;
    let mut ruins_level: Option<f64> = None;
    let mut shape_seed: Option<u64> = None;
    let mut decor_seed: Option<u64> = None;
    let mut name_seed: Option<u64> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .unwrap_or_else(|| fail(&format!("{name} requires a value")))
        };
        match arg.as_str() {
            "-c" | "--config" => config_path = Some(value("--config")),
            "-m" | "--mode" => {
                mode = Some(match value("--mode").to_ascii_lowercase().as_str() {
                    "cave" => Mode::Cave,
                    "forest" | "glade" => Mode::Forest,
                    other => fail(&format!("unknown mode: {other} (cave|forest)")),
                });
            }
            "-g" | "--grid" => {
                grid = Some(match value("--grid").to_ascii_lowercase().as_str() {
                    "hex" => GridStyle::Hex,
                    "square" => GridStyle::Square,
                    "none" => GridStyle::None,
                    other => fail(&format!("unknown grid style: {other} (hex|square|none)")),
                });
            }
            "-s" | "--seed" => {
                seed = Some(
                    value("--seed")
                        .parse()
                        .unwrap_or_else(|_| fail("--seed must be an unsigned integer")),
                );
            }
            "-t" | "--tags" => {
                tags = Some(Tags::parse(&value("--tags")).unwrap_or_else(|e| fail(&e)));
            }
            "--shape-seed" | "--decor-seed" | "--name-seed" => {
                let parsed: u64 = value(&arg).parse().unwrap_or_else(|_| {
                    fail(&format!("{arg} must be an unsigned integer"));
                });
                match arg.as_str() {
                    "--shape-seed" => shape_seed = Some(parsed),
                    "--decor-seed" => decor_seed = Some(parsed),
                    _ => name_seed = Some(parsed),
                }
            }
            "-w" | "--water" => {
                let level: f64 = value("--water")
                    .parse()
                    .unwrap_or_else(|_| fail("--water must be a number"));
                if !(0.0..=1.0).contains(&level) {
                    fail("--water must be between 0.0 and 1.0");
                }
                water_level = Some(level);
            }
            "-r" | "--ruins" => {
                let level: f64 = value("--ruins")
                    .parse()
                    .unwrap_or_else(|_| fail("--ruins must be a number"));
                if !(0.0..=1.0).contains(&level) {
                    fail("--ruins must be between 0.0 and 1.0");
                }
                ruins_level = Some(level);
            }
            "-o" | "--out" => out = Some(value("--out")),
            "-d" | "--debug" => debug = Some(true),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other if !other.starts_with('-') && config_path.is_none() => {
                config_path = Some(other.to_string());
            }
            other => fail(&format!("unknown argument: {other}\n{USAGE}")),
        }
    }

    // Explicit config must exist; the maps.toml fallback is best-effort.
    let config = match &config_path {
        Some(p) => Config::load(Path::new(p)).unwrap_or_else(|e| fail(&e)),
        None if Path::new("maps.toml").exists() => {
            Config::load(Path::new("maps.toml")).unwrap_or_else(|e| fail(&e))
        }
        None => Config::default(),
    };

    // Precedence: defaults < config file < command-line flags.
    let seed = seed.or(config.seed).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    });
    let tags = match tags {
        Some(t) => Some(t),
        None => config.tags().unwrap_or_else(|e| fail(&e)),
    };
    let out = out
        .or_else(|| config.output.clone())
        .unwrap_or_else(|| "cave.svg".to_string());
    let debug = debug.or(config.debug).unwrap_or(false);
    let mode = mode
        .or(config.mode.map(Mode::from))
        .unwrap_or(Mode::Cave);
    let grid = grid
        .or(config.grid.map(GridStyle::from))
        .unwrap_or(GridStyle::Hex);

    let map = generate_with(
        seed,
        &GenOptions {
            mode,
            grid,
            tags,
            outline: config.outline_params(),
            water_level: water_level.or(config.water_level),
            ruins_level: ruins_level.or(config.ruins_level),
            shape_seed: shape_seed.or(config.shape_seed),
            decor_seed: decor_seed.or(config.decor_seed),
            name_seed: name_seed.or(config.name_seed),
        },
    );
    let rendered = if debug { debug_svg(&map) } else { svg(&map) };
    if let Err(e) = std::fs::write(&out, rendered) {
        fail(&format!("failed to write {out}: {e}"));
    }
    println!(
        "\"{}\" | seed {} (shape {}, decor {}, name {}) | tags: {} | {} areas -> {}",
        map.title,
        map.seed,
        map.shape_seed,
        map.decor_seed,
        map.name_seed,
        map.tags,
        map.areas.count(),
        out
    );
}
