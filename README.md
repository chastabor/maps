# maps

A procedural fantasy map generator in the spirit of
[watabou's Cave Generator](https://watabou.itch.io/cave-generator), written in
Rust. It produces hand-drawn-style **cave systems** and **forest glades** as
SVG files, driven by a TOML configuration file and/or command-line flags.

The workspace has two parts:

| Crate | Purpose |
|---|---|
| `maps` (repo root) | The `maps` CLI binary and config handling |
| `crates/maps-core` | The pure, deterministic generation engine (no I/O; wasm-friendly) |
| `crates/maps-wasm` | wasm-bindgen wrapper powering the `web/` demo page |

How generation works — the deterministic pipeline and the ruins extension —
is described in [DESIGN.md](DESIGN.md).

## Building

```sh
cargo build --release          # binary at target/release/maps
cargo test --workspace         # run the full test suite
```

## Usage

```sh
maps [OPTIONS] [CONFIG]
```

Options come from three layers, later winning over earlier:
**built-in defaults → TOML config file → command-line flags.**
The config file is the positional argument (or `-c/--config`); if none is
given and `./maps.toml` exists, it is loaded automatically.

```sh
maps examples/cave.toml               # generate from a config
maps examples/cave.toml -m forest     # same map, re-dressed as a glade
maps -s 42 -t large,hub,wet -o my.svg # config-free, flags only
```

### Command-line flags

| Flag | Meaning |
|---|---|
| `-c, --config <FILE>` | Config file path (same as the positional argument) |
| `-m, --mode <MODE>` | `cave` (default) or `forest` (alias `glade`) |
| `-g, --grid <STYLE>` | Grid overlay: `hex` (default), `square`, or `none` |
| `-s, --seed <N>` | Master RNG seed (default: derived from the clock) |
| `--shape-seed <N>` | Re-roll/pin just the map shape (outline, water, stones) |
| `--decor-seed <N>` | Re-roll/pin just the hatch fans / tree canopies |
| `--name-seed <N>` | Re-roll/pin just the title |
| `--title <NAME>` | Use this exact map title instead of generating one |
| `-t, --tags <LIST>` | Comma-separated tags (see below) |
| `-w, --water <LEVEL>` | Water level `0.0..=1.0` (0 = dry, 1 = fully submerged) |
| `-r, --ruins <LEVEL>` | Ruins level `0.0..=1.0`: fraction of areas that become geometric |
| `-o, --out <FILE>` | Output SVG path (default `cave.svg`) |
| `-d, --debug` | Render raw hex cells instead of the finished map |
| `-h, --help` | Show help |

## Configuration file

All fields are optional; unknown fields are rejected. See `maps.toml` at the
repo root for a commented example, and `examples/` for ready-to-run configs.

```toml
# Map type: "cave" (default) or "forest" (alias "glade").
mode = "cave"

# Grid overlay on the floor: "hex" (default, the native lattice), "square"
# (lines sized to meet the hex centres of every other row), or "none".
grid = "hex"

# Master RNG seed; omit for a clock-derived random seed.
seed = 42

# Optional per-stream sub-seed overrides (see "Replicating a map" below).
#shape_seed = 123   # map outline, water, stones
#decor_seed = 456   # hatch fans / tree canopies
#name_seed = 789    # the title

# Generation tags: a comma-separated string or an array of strings.
tags = "large,hub,wet"        # or: tags = ["large", "hub", "wet"]

# Water level, 0.0..1.0: the fraction of the terrain that floods, lowest
# basins first (0.5 = lowest half under water, 1 = fully submerged). This
# fine-tunes the water tag's default level — wet 0.45, untagged 0.15; the
# dry tag always means no water and ignores it. Pools get a darker
# deep-water band and a mud fringe automatically.
water_level = 0.4

# Ruins level, 0.0..1.0: the fraction of areas regrown as geometric ruins.
# Rooms become rectangles (straight walls) or circles (arching walls);
# corridors become straight or arcing halls (ones too contorted to fit, or
# whose reshaping would orphan a door, stay organic). Ruins growing into
# contact merge into one larger space, with organic seams between them.
# This fine-tunes the ruins tag's default fraction — ruins 0.5, untagged
# 0.1; the organic tag always means no ruins and ignores it.
#ruins_level = 0.5

# Output SVG path.
output = "cave.svg"

# Render the raw hex-cell debug view instead of the finished map.
debug = false

# Outline / smoothing overrides (defaults shown).
[outline]
hex_size = 12.0      # pixel size of one hex cell
bumpiness = 0.55     # boundary smoothing strength, 0..1
smooth_passes = 1    # number of smoothing passes
irregularity = 0.16  # vertex jitter, as a fraction of hex size
roughness = 0.07     # fine jitter for the subdivision rounds
narrow_pull = 0.4    # how tightly tunnels are pinched, 0..1
chaikin_iters = 2    # corner-cutting iterations
```

## Tags

Tags act as presets/switches for the generator, one per group (later tags in
a list replace earlier ones from the same group). Omitted groups are chosen
randomly from the seed.

| Group | Tag | Effect |
|---|---|---|
| **Size** | `small` | 2–3 areas |
| | `medium` | 3–8 areas |
| | `large` | 9–19 areas |
| **Layout** | `hub` | One large central chamber (60–79 cells), small satellites |
| | `chamber` | Evenly sized rooms |
| | `burrow` | Highly varied room sizes, more corridors |
| **Shape** | `cavities` | Round, blobby areas |
| | `coral` | Narrow branching tendrils |
| | `chaotic` | Irregular growth favouring 2-connection cells |
| **Links** | `tree` | Doorways form a spanning tree — no loops |
| | `connected` | Loops allowed, but triangles of adjacent areas are broken |
| **Exits** | `sealed` | No exits to the outside |
| | `entrance` | Exactly 1 exit |
| | `passage` | 2 exits |
| | `junction` | 3–4 exits |
| **Water** | `wet` | Floods ~45% of the terrain |
| | `dry` | No water at all |
| **Ruins** | `ruins` | Half the areas become geometric ruins, regrown on the hex grid — rooms turn into rectangles (straight walls) or circles (arching walls), corridors into straight or arcing halls. Ruins that grow into contact merge into one larger space. Ruin walls get their own decoration: faded stipple dots in caves, overlapping masonry tiles in glades. Tune the fraction with `ruins_level` / `--ruins`. |
| | `organic` | No ruins at all (untagged maps allow the occasional ruin, ~10%) |
| **Pattern** | `mosaic` | Ruin floors tiled with wave-shaded hex mosaic (grout lines between tiles) |
| | `truchet` | Ruin floors traced with flowing Truchet ribbons (knots or sweeping mazes per ruin) |
| | `islamic` | Ruin floors laced with Hankin polygons-in-contact star lines (6-point stars/rosettes) |
| | `plain` | Bare ruin floors (the most common seed roll) |

Example: `maps -t large,burrow,tree,junction,wet` or `maps -t large,chamber,ruins -r 0.8`

## Replicating a map exactly

Randomness is split into three independent streams, each derived from the
master seed but individually overridable:

1. **shape** — tags (when random), area growth, doorways/corridors/exits,
   boundary smoothing, water, stones
2. **decor** — the wall hatch fans (cave) or border tree canopies (forest)
3. **name** — the title

After every run, the binary prints the effective value of all three:

```
"Blackmaw Mire" | seed 42 (shape 13679457532755275413, decor 2949826092126892291, name 5139283748462763858) | tags: large hub wet | 11 areas -> examples/cave.svg
```

To reproduce that exact image on another system, supply the three sub-seeds
plus the same tags and options — the master seed is then irrelevant:

```toml
tags = "large,hub,wet"
water_level = 0.4
shape_seed = 13679457532755275413
decor_seed = 2949826092126892291
name_seed = 5139283748462763858
```

Because the streams are independent, you can also pin two and re-roll one —
keep a shape you like while trying different hatching or titles:

```sh
maps examples/cave.toml --decor-seed 7   # same cave, different hatching
maps examples/cave.toml --name-seed 9    # same cave, different title
```

Switching `mode` between `cave` and `forest` preserves the shape too: the
same seed renders the identical outline, pools and stones in either dressing.

## Examples

```sh
maps examples/cave.toml         # cave          -> examples/cave.svg
maps examples/forest.toml       # glade         -> examples/forest.svg
maps examples/ruins.toml        # ruined cave   -> examples/ruins.svg
maps examples/ruins-glade.toml  # ruined glade, mosaic floors -> examples/ruins-glade.svg
maps examples/pattern.toml      # tiled ruined cave (truchet) -> examples/pattern.svg
maps examples/pattern-glade.toml # ruined glade (islamic)     -> examples/pattern-glade.svg
maps examples/dungeon.toml      # clean doored dungeon rooms  -> examples/dungeon.svg
```

The ruins tag works in both modes: in a cave the geometry reads as ruined
architecture swallowed by the cavern, its walls shaded with faded stipple
dots; in a forest, as the overgrown foundations of a lost settlement edged
with courses of masonry tile. Ruins are regrown on the hex grid like any
other node, so two that grow into contact merge into one larger space —
joined by full-width, organically decorated seams.

## Web demo

A static page in `web/` drives the generator compiled to WebAssembly:
generate/re-roll buttons (per randomness stream), radio groups for every tag
family, mode/grid selectors, water/ruins level sliders, SVG download, and a
permalink in the URL hash that replicates the exact map anywhere.

Build with the standard toolchain (no wasm-pack needed):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked   # match Cargo.lock
cargo install wasm-opt --locked

cargo build -p maps-wasm --target wasm32-unknown-unknown --profile wasm-release
wasm-bindgen target/wasm32-unknown-unknown/wasm-release/maps_wasm.wasm \
  --target web --out-dir web/pkg

# Size-optimization pass: shrinks web/pkg/maps_wasm_bg.wasm in place (~25%
# smaller, e.g. 324 KB -> 237 KB). -Os optimizes for size; -Oz squeezes harder.
wasm-opt -Os web/pkg/maps_wasm_bg.wasm -o web/pkg/maps_wasm_bg.wasm

python3 -m http.server -d web   # then open http://localhost:8000
```

The `wasm-opt` pass is optional — the page runs fine without it — but it
noticeably reduces the download size; run it before deploying. It rewrites the
`.wasm` in place, so re-run it after every `wasm-bindgen` regeneration.

The wasm output is byte-identical to the native CLI for the same seeds
(pure-Rust PRNG, IEEE f64), so permalinks, config files and the printed
seed line all describe the same map. Note: seeds are `u64` and cross the
JS boundary as **strings** — JavaScript numbers corrupt integers above 2^53.

## Determinism notes

- The same seeds, tags and options produce byte-identical SVG output across
  platforms (pure-Rust PRNG, no platform randomness).
- Changing style parameters such as `[outline] hex_size` alters stroke
  sampling counts and therefore downstream random details — pin the streams
  you care about if you tweak style while iterating.

## License

MIT — see [LICENSE](LICENSE).

## Credits

- Generation approach modelled on watabou's
  [Cave Generator](https://watabou.itch.io/cave-generator), reimplemented
  from public descriptions.
- Algorithm reference: Boris the Brave's
  [How does Cave/Glade Generator work?](https://www.boristhebrave.com/2023/11/19/how-does-cave-glade-generator-work/)
