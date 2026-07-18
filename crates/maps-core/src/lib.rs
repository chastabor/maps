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

pub struct CaveMap {
    pub seed: u64,
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
    pub title: String,
}

/// Everything that shapes generation besides the seed.
#[derive(Clone, Debug, Default)]
pub struct GenOptions {
    pub mode: Mode,
    /// `None` picks random tags from the seed.
    pub tags: Option<Tags>,
    pub outline: OutlineParams,
    /// Water level as a fill fraction in 0..=1: 0 is completely dry, 0.5
    /// floods the lowest half of the terrain, 1 submerges everything.
    /// `None` uses the tag default (wet ~0.45, dry 0, otherwise ~0.15).
    pub water_level: Option<f64>,
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

/// Generate a map with explicit options.
pub fn generate_with(seed: u64, opts: &GenOptions) -> CaveMap {
    let mode = opts.mode;
    let oparams = &opts.outline;
    let mut rng = Pcg64::seed_from_u64(seed);
    let tags = opts.tags.clone().unwrap_or_else(|| Tags::random(&mut rng));
    let params = resolve(&tags, &mut rng);
    let grid = HexGrid::hexagon(grid_radius(&params));
    let mut areas = grow_areas(&grid, &mut rng, &params);
    let topology = topology::build(&grid, &mut areas, &tags, &mut rng);
    let outline = build_outline(&areas, &topology, oparams, &mut rng);
    let w = water::build_water(&areas, &topology, oparams, &tags, opts.water_level, &mut rng);
    let (floor, narrow) = outline::floor_and_narrow(&areas, &topology);
    let stones = decor::stones(&floor, &narrow, &w.cells, oparams.hex_size, &mut rng);
    let (hatching, trees) = match mode {
        Mode::Cave => (decor::hatching(&outline, &mut rng), Vec::new()),
        Mode::Forest => (Vec::new(), decor::trees(&outline, &mut rng)),
    };
    let title = naming::title(&mut rng, !w.pools.is_empty(), mode);
    CaveMap {
        seed,
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
        title,
    }
}
