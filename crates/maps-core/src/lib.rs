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
pub mod tags;
pub mod topology;
pub mod water;

use grid::HexGrid;
use growth::{Areas, GrowthParams, grid_radius, grow_areas, resolve};
use rand::SeedableRng;
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
    pub title: String,
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
    /// `None` uses the tag default (wet ~0.45, dry 0, otherwise ~0.15).
    pub water_level: Option<f64>,
    /// Fraction (0..=1) of the non-corridor areas that take on geometric
    /// ruin shapes (rectangles/circles) in place of their organic outline.
    /// `None` uses the tag default (`ruins` tag = 0.5, otherwise 0).
    pub ruins_level: Option<f64>,
    /// Override the shape stream (tags, areas, topology, outline, water,
    /// stones). Defaults to a sub-seed derived from the master seed.
    pub shape_seed: Option<u64>,
    /// Override the decoration stream (hatch fans / tree canopies).
    pub decor_seed: Option<u64>,
    /// Override the naming stream (the title).
    pub name_seed: Option<u64>,
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

/// Derive a stream-specific sub-seed from the master seed (splitmix64).
fn sub_seed(seed: u64, stream: u64) -> u64 {
    let mut z = seed.wrapping_add((stream + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
    let grid = HexGrid::hexagon(grid_radius(&params));
    let mut areas = grow_areas(&grid, &mut rng, &params);
    let topology = topology::build(&grid, &mut areas, &tags, &mut rng);
    let ruins_level = opts.ruins_level.unwrap_or(match tags.ruins {
        Some(tags::RuinsTag::Ruins) => 0.5,
        Some(tags::RuinsTag::Organic) => 0.0,
        None => 0.1,
    });
    // Reshapes the selected areas' cells to the rasterized geometry, so all
    // downstream layers (outline, water, stones, decor) see the real
    // footprint and touching shapes union at the cell level.
    let ruin_shapes = ruins::build(
        &mut areas,
        &topology,
        &grid,
        ruins_level,
        oparams.hex_size,
        &mut rng,
    );
    let ruin_map = ruins::ruin_cell_map(&areas, &ruin_shapes);
    let outline = build_outline(&areas, &topology, &ruin_map, oparams, &mut rng);
    let w = water::build_water(&areas, &topology, oparams, &tags, opts.water_level, &mut rng);
    let (floor, narrow) = outline::floor_and_narrow(&areas, &topology);
    let stones = decor::stones(&floor, &narrow, &w.cells, oparams.hex_size, &mut rng);

    let ruin_cells: std::collections::HashSet<grid::Hex> = ruin_map.keys().copied().collect();

    let mut decor_rng = Pcg64::seed_from_u64(decor_seed);
    let (hatching, dots, trees, tiles) = match mode {
        Mode::Cave => {
            let (fans, dots) =
                decor::hatching(&outline, &ruin_cells, oparams.hex_size, &mut decor_rng);
            (fans, dots, Vec::new(), Vec::new())
        }
        Mode::Forest => {
            let (trees, tiles) =
                decor::trees(&outline, &ruin_cells, oparams.hex_size, &mut decor_rng);
            (Vec::new(), Vec::new(), trees, tiles)
        }
    };

    let mut name_rng = Pcg64::seed_from_u64(name_seed);
    let title = naming::title(&mut name_rng, !w.pools.is_empty(), mode);
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
        title,
    }
}
