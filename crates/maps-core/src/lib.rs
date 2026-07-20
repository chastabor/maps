//! maps-core: procedural cave (and, later, glade/forest) map generator in
//! the style of watabou's Cave/Glade Generator. Pure and deterministic:
//! `(seed, tags)` in, map out. No I/O, no platform dependencies — usable
//! natively and from wasm.

pub mod decor;
pub mod grid;
pub mod growth;
pub mod naming;
pub mod outline;
pub mod render;
pub mod ruins;
pub mod symmetry;
pub mod tags;
pub mod topology;
pub mod water;

use grid::HexGrid;
use growth::{Areas, GrowthParams, grid_radius, grow_areas, resolve};
use rand::Rng;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_pcg::Pcg64;
use outline::{OutlineParams, Point, build_outline};
use tags::Tags;
use topology::Topology;

/// What the generated space represents: a cave system (walls, hatching) or
/// a forest glade (clearings ringed by trees).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Cave,
    Forest,
}

/// The grid overlay drawn on the floor: the native hex lattice, a square
/// grid sized so its lines meet the hex centres of every other row, or no
/// grid at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GridStyle {
    #[default]
    Hex,
    Square,
    None,
}

/// The architectural state of an area, on the organic → ruin → dungeon
/// spectrum. `ruins_level` sets how much of the map leaves `Organic`; of that
/// geometric remainder, `dungeon_level` promotes a fraction from `Ruin`
/// (weathered geometry) to `Dungeon` (clean walls, doors, symmetry).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AreaKind {
    #[default]
    Organic,
    Ruin,
    Dungeon,
}

impl AreaKind {
    /// Whether areas of these two kinds may sit cell-adjacent with no rock gap
    /// between them. Only two **dungeon** areas fuse — into one compound room
    /// (e.g. a rectangle with an attached circular silo); every other pair
    /// keeps the one-cell doorway gap. Growth, reshaping and symmetry all
    /// consult this one rule.
    pub fn may_fuse(self, other: AreaKind) -> bool {
        self == AreaKind::Dungeon && other == AreaKind::Dungeon
    }
}

/// How one door onto a dungeon room is drawn. All three are a hex-aligned bar
/// across the doorway with jamb caps at each end; they differ in the leaf:
/// `Wood` is a plain leaf, `Metal` adds a reinforcing band down its length,
/// `Portcullis` is a row of bars (drawn as circles) instead of a leaf. One per
/// `topology` door; entries for doors that touch no dungeon area are unused.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DoorStyle {
    #[default]
    Wood,
    Metal,
    Portcullis,
}

pub struct CaveMap {
    /// The master seed the sub-seeds were derived from.
    pub seed: u64,
    /// Grid overlay style for rendering.
    pub grid_style: GridStyle,
    /// The three effective sub-seeds; quote these to replicate or remix the
    /// map ([`GenOptions::shape_seed`] and friends).
    pub shape_seed: u64,
    pub decor_seed: u64,
    pub name_seed: u64,
    pub mode: Mode,
    pub tags: Tags,
    pub params: GrowthParams,
    pub grid: HexGrid,
    pub areas: Areas,
    pub topology: Topology,
    /// Smoothed floor boundary loops (outer walls and interior pillars).
    pub outline: Vec<Vec<Point>>,
    /// Smoothed water pool loops at the waterline.
    pub water: Vec<Vec<Point>>,
    /// Deep-water band inside the pools (terrain well below the level).
    pub deep_water: Vec<Vec<Point>>,
    /// Mud fringe just above the waterline, ringing the pools.
    pub mud: Vec<Vec<Point>>,
    /// Rubble stone polygons.
    pub stones: Vec<Vec<Point>>,
    /// Wall hatching: five-stroke cone fans along the wall, each with an
    /// opaque footprint that hides fans beneath it; clipped by the floor at
    /// render time (cave mode).
    pub hatching: Vec<decor::HatchFan>,
    /// Border tree canopies as star polygons with a depth band
    /// (0 = nearest the clearing, rendered lightest) (forest mode).
    pub trees: Vec<(Vec<Point>, usize)>,
    /// Faded stipple dots along ruin walls as (centre, radius, opacity)
    /// discs — larger and darker near the wall (cave mode).
    pub dots: Vec<(Point, f64, f64)>,
    /// Masonry tiles along ruin walls (forest mode).
    pub tiles: Vec<Vec<Point>>,
    /// Per-area geometric ruin shape, if the area was reshaped.
    pub ruins: Vec<Option<ruins::RuinShape>>,
    /// Per-door glyph style, aligned with `topology.doors`. Only doors that
    /// touch a `Dungeon` area are rendered; the rest carry default entries.
    /// (Per-area kinds live on `areas` — see [`growth::Areas::kind`].)
    pub door_styles: Vec<DoorStyle>,
    /// Floor tile pattern elements on ruin-area cells (pattern tag).
    pub floor_pattern: Vec<decor::PatternElem>,
    pub title: String,
}

impl CaveMap {
    /// Whether the area at `i` is a clean, doored dungeon room (as opposed to
    /// organic or weathered ruin). Out-of-range indices are not dungeons.
    pub fn is_dungeon(&self, i: usize) -> bool {
        self.areas.kinds().get(i) == Some(&AreaKind::Dungeon)
    }
}

/// Everything that shapes generation besides the seed.
#[derive(Clone, Debug, Default)]
pub struct GenOptions {
    pub mode: Mode,
    /// Grid overlay style (default: hex).
    pub grid: GridStyle,
    /// `None` picks random tags from the seed.
    pub tags: Option<Tags>,
    pub outline: OutlineParams,
    /// Water level as a fill fraction in 0..=1: 0 is completely dry, 0.5
    /// floods the lowest half of the terrain, 1 submerges everything.
    /// Fine-tunes the water tag's default (wet 0.45, untagged 0.15); the
    /// `dry` tag always means no water and ignores this.
    pub water_level: Option<f64>,
    /// Fraction (0..=1) of the non-corridor areas that take on geometric
    /// ruin shapes (rectangles/circles) in place of their organic outline.
    /// Fine-tunes the ruins tag's default (ruins 0.5, untagged 0.1); the
    /// `organic` tag always means no ruins and ignores this.
    pub ruins_level: Option<f64>,
    /// Fraction (0..=1) of the geometric (ruin) areas promoted to clean
    /// **dungeon** rooms — crisp walls, rendered doors, and (later) symmetric
    /// wings — instead of weathered ruins. Nested inside `ruins_level`: with
    /// no geometric areas it does nothing. Fine-tunes the dungeon tag's
    /// default (dungeon 0.6, untagged 0.0); the `natural` tag forces 0.
    pub dungeon_level: Option<f64>,
    /// Override the shape stream (tags, areas, topology, outline, water,
    /// stones). Defaults to a sub-seed derived from the master seed.
    pub shape_seed: Option<u64>,
    /// Override the decoration stream (hatch fans / tree canopies).
    pub decor_seed: Option<u64>,
    /// Override the naming stream (the title).
    pub name_seed: Option<u64>,
    /// Use this exact title instead of generating one (empty/whitespace is
    /// ignored). The name stream is left untouched, so seeded naming
    /// resumes when the override is removed.
    pub title: Option<String>,
}

/// Generate a cave map. `tags: None` picks random tags from the seed.
pub fn generate(seed: u64, tags: Option<Tags>) -> CaveMap {
    generate_with(
        seed,
        &GenOptions {
            tags,
            ..GenOptions::default()
        },
    )
}

/// The tags a master seed rolls when none are supplied — the same
/// derivation `generate_with` uses, exposed so UIs can preview/edit them.
pub fn random_tags_for(seed: u64) -> Tags {
    let shape_seed = sub_seed(seed, 0);
    let mut tag_rng = Pcg64::seed_from_u64(sub_seed(shape_seed, 3));
    Tags::random(&mut tag_rng)
}

/// Derive a stream-specific sub-seed from the master seed (splitmix64).
fn sub_seed(seed: u64, stream: u64) -> u64 {
    let mut z = seed.wrapping_add((stream + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Assign a kind to each of the `n` area slots *before* growth, so growth can
/// react (only dungeon areas may fuse). `round(ruins_level·n)` slots become
/// geometric (ruin ∪ dungeon); of those, `round(dungeon_level·n_geo)` are
/// dungeon and the rest ruin; all remaining slots are organic. Which slots take
/// which kind is a shape-stream shuffle. With no geometric areas it draws
/// nothing, so a fully-organic map's growth is untouched.
fn classify_slots<R: Rng>(n: usize, ruins_level: f64, dungeon_level: f64, rng: &mut R) -> Vec<AreaKind> {
    let n_geo = ((ruins_level.clamp(0.0, 1.0) * n as f64).round() as usize).min(n);
    if n_geo == 0 {
        return vec![AreaKind::Organic; n];
    }
    let n_dun = ((dungeon_level.clamp(0.0, 1.0) * n_geo as f64).round() as usize).min(n_geo);
    let mut slots: Vec<usize> = (0..n).collect();
    slots.shuffle(rng);
    let mut kinds = vec![AreaKind::Organic; n];
    for &s in &slots[..n_dun] {
        kinds[s] = AreaKind::Dungeon;
    }
    for &s in &slots[n_dun..n_geo] {
        kinds[s] = AreaKind::Ruin;
    }
    kinds
}

/// Generate a map with explicit options.
///
/// Randomness is split into three independent streams so each can be
/// re-rolled without disturbing the others: shape, decoration and name.
/// With all three (or just the master seed) plus the same tags/options, the
/// image replicates identically anywhere.
pub fn generate_with(seed: u64, opts: &GenOptions) -> CaveMap {
    let mode = opts.mode;
    let oparams = &opts.outline;
    let shape_seed = opts.shape_seed.unwrap_or_else(|| sub_seed(seed, 0));
    let decor_seed = opts.decor_seed.unwrap_or_else(|| sub_seed(seed, 1));
    let name_seed = opts.name_seed.unwrap_or_else(|| sub_seed(seed, 2));

    // Random tags come from their own stream (derived from the shape seed)
    // so that supplying the same tags explicitly — as a replica must —
    // leaves the shape stream untouched and reproduces the identical map.
    let tags = opts.tags.clone().unwrap_or_else(|| {
        let mut tag_rng = Pcg64::seed_from_u64(sub_seed(shape_seed, 3));
        Tags::random(&mut tag_rng)
    });
    let mut rng = Pcg64::seed_from_u64(shape_seed);
    let params = resolve(&tags, &mut rng);
    // The tag picks the state; the level only fine-tunes it: organic always
    // means no ruins, while ruins/untagged use the override in place of
    // their default fraction.
    let ruins_level = match tags.ruins {
        Some(tags::RuinsTag::Organic) => 0.0,
        Some(tags::RuinsTag::Ruins) => opts.ruins_level.unwrap_or(0.5),
        None => opts.ruins_level.unwrap_or(0.1),
    };
    // Nested inside ruins: the fraction of the geometric areas that become
    // clean dungeon rooms rather than weathered ruins. `natural` forces none;
    // untagged is none unless overridden.
    let dungeon_level = match tags.dungeon {
        Some(tags::DungeonTag::Natural) => 0.0,
        Some(tags::DungeonTag::Dungeon) => opts.dungeon_level.unwrap_or(0.6),
        None => opts.dungeon_level.unwrap_or(0.0),
    };
    // Classify each area SLOT before growth, so growth can react to it (only
    // dungeon areas may fuse). Classification, growth, reshaping and doors all
    // draw from the one shape stream — determinism is per-seed, with no
    // byte-compatibility to older output.
    let slot_kinds = classify_slots(params.sizes.len(), ruins_level, dungeon_level, &mut rng);
    let grid = HexGrid::hexagon(grid_radius(&params));
    // Staggered simultaneous growth: dungeon rooms grow as their geometry and
    // symmetric wings grow as lockstep sibling orbits (symmetry is chosen
    // inside, from the shape stream).
    let mut areas = grow_areas(&grid, &mut rng, &params, &slot_kinds, oparams.hex_size);
    let topology = topology::build(&grid, &mut areas, &tags, &mut rng);
    // Reshape the ruin areas to their rasterized geometry, so all downstream
    // layers (outline, water, stones, decor) see the real footprint and
    // touching shapes union at the cell level. Ruins that can't reshape are
    // demoted back to organic inside; dungeon rooms grew as their geometry.
    ruins::build(&mut areas, &topology, &grid, oparams.hex_size, &mut rng);
    let ruin_map = ruins::ruin_cell_map(&areas, oparams.hex_size);

    // Style each door that opens onto a dungeon room; other doors keep a
    // default entry and draw nothing.
    let door_styles: Vec<DoorStyle> = topology
        .doors
        .iter()
        .map(|d| {
            if areas.kind(d.a) == AreaKind::Dungeon || areas.kind(d.b) == AreaKind::Dungeon {
                match rng.random_range(0..100) {
                    0..22 => DoorStyle::Metal,
                    22..37 => DoorStyle::Portcullis,
                    _ => DoorStyle::Wood,
                }
            } else {
                DoorStyle::default()
            }
        })
        .collect();
    // Dungeon-area cells: their walls take the clean treatment, so they are
    // held out of the weathered ruin decor (stipple / masonry) below.
    let dungeon_cells: std::collections::HashSet<grid::Hex> = (0..areas.count())
        .filter(|&i| areas.kind(i) == AreaKind::Dungeon)
        .flat_map(|i| areas.cells[i].iter().copied())
        .collect();

    let outline = build_outline(&areas, &topology, &ruin_map, &dungeon_cells, oparams, &mut rng);
    let w = water::build_water(&areas, &topology, oparams, &tags, opts.water_level, &mut rng);
    let (floor, narrow) = outline::floor_and_narrow(&areas, &topology);
    let stones = decor::stones(&floor, &narrow, &w.cells, oparams.hex_size, &mut rng);

    // Only weathered (ruin) walls get stipple/masonry; dungeon cells are
    // excluded here and their walls skipped in decor, leaving a clean line.
    let ruin_cells: std::collections::HashSet<grid::Hex> = ruin_map
        .keys()
        .copied()
        .filter(|h| !dungeon_cells.contains(h))
        .collect();

    let mut decor_rng = Pcg64::seed_from_u64(decor_seed);
    let (hatching, dots, trees, tiles) = match mode {
        Mode::Cave => {
            let (fans, dots) = decor::hatching(
                &outline,
                &ruin_cells,
                &dungeon_cells,
                oparams.hex_size,
                &mut decor_rng,
            );
            (fans, dots, Vec::new(), Vec::new())
        }
        Mode::Forest => {
            let (trees, tiles) = decor::trees(
                &outline,
                &ruin_cells,
                &dungeon_cells,
                oparams.hex_size,
                &mut decor_rng,
            );
            (Vec::new(), Vec::new(), trees, tiles)
        }
    };
    // Floor tiles on every geometric area — weathered ruins and clean dungeon
    // rooms alike — after the other decor so `plain` maps keep their exact
    // output. One sorted cell list per area.
    let pattern_tag = tags.pattern.unwrap_or(tags::PatternTag::Plain);
    let ruin_area_cells: Vec<Vec<grid::Hex>> = (0..areas.count())
        .filter(|&i| areas.kind(i) != AreaKind::Organic)
        .map(|i| {
            let mut v = areas.cells[i].clone();
            v.sort_unstable();
            v
        })
        .collect();
    let floor_pattern =
        decor::floor_pattern(&ruin_area_cells, pattern_tag, oparams.hex_size, &mut decor_rng);

    // Snapshot the shapes before `areas` moves into the map.
    let ruin_shapes = areas.shapes().to_vec();
    let title = match opts.title.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            let mut name_rng = Pcg64::seed_from_u64(name_seed);
            naming::title(&mut name_rng, !w.pools.is_empty(), mode)
        }
    };
    CaveMap {
        seed,
        grid_style: opts.grid,
        shape_seed,
        decor_seed,
        name_seed,
        mode,
        tags,
        params,
        grid,
        areas,
        topology,
        outline,
        water: w.pools,
        deep_water: w.deep,
        mud: w.mud,
        stones,
        hatching,
        trees,
        dots,
        tiles,
        ruins: ruin_shapes,
        door_styles,
        floor_pattern,
        title,
    }
}
