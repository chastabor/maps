//! TOML configuration for map generation. Every field is optional; the
//! defaults match `maps-core`'s own.

use maps_core::Mode;
use maps_core::outline::OutlineParams;
use maps_core::tags::Tags;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Map type: "cave" (default) or "forest" (alias "glade").
    pub mode: Option<ModeSpec>,
    /// RNG seed; omit for a clock-derived seed.
    pub seed: Option<u64>,
    /// Generation tags, either `"large,hub,wet"` or `["large", "hub", "wet"]`.
    pub tags: Option<TagsSpec>,
    /// Output SVG path (default `cave.svg`).
    pub output: Option<String>,
    /// Render the raw hex-cell debug view instead of the finished map.
    pub debug: Option<bool>,
    /// Water level in 0..=1: 0 is no water, 1 submerges the whole map, and
    /// values between flood that fraction of the terrain, lowest basins
    /// first. Overrides the wet/dry tag default.
    pub water_level: Option<f64>,
    /// Ruins level in 0..=1: the fraction of areas that take on geometric
    /// shapes (rectangles/circles) instead of their organic outline.
    /// Overrides the ruins tag default (0.5).
    pub ruins_level: Option<f64>,
    /// Override the shape sub-seed (map outline, water, stones).
    pub shape_seed: Option<u64>,
    /// Override the decoration sub-seed (hatch fans / tree canopies).
    pub decor_seed: Option<u64>,
    /// Override the naming sub-seed (the title).
    pub name_seed: Option<u64>,
    /// Outline/smoothing overrides.
    pub outline: Option<OutlineConfig>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ModeSpec {
    Cave,
    Forest,
    Glade,
}

impl From<ModeSpec> for Mode {
    fn from(m: ModeSpec) -> Mode {
        match m {
            ModeSpec::Cave => Mode::Cave,
            ModeSpec::Forest | ModeSpec::Glade => Mode::Forest,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum TagsSpec {
    List(Vec<String>),
    Text(String),
}

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct OutlineConfig {
    pub hex_size: Option<f64>,
    pub bumpiness: Option<f64>,
    pub smooth_passes: Option<usize>,
    pub irregularity: Option<f64>,
    pub roughness: Option<f64>,
    pub narrow_pull: Option<f64>,
    pub chaikin_iters: Option<usize>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("invalid config {}: {e}", path.display()))
    }

    pub fn tags(&self) -> Result<Option<Tags>, String> {
        match &self.tags {
            None => Ok(None),
            Some(TagsSpec::Text(s)) => Tags::parse(s).map(Some),
            Some(TagsSpec::List(v)) => Tags::parse(&v.join(",")).map(Some),
        }
    }

    pub fn outline_params(&self) -> OutlineParams {
        let mut p = OutlineParams::default();
        if let Some(o) = &self.outline {
            if let Some(v) = o.hex_size {
                p.hex_size = v;
            }
            if let Some(v) = o.bumpiness {
                p.bumpiness = v;
            }
            if let Some(v) = o.smooth_passes {
                p.smooth_passes = v;
            }
            if let Some(v) = o.irregularity {
                p.irregularity = v;
            }
            if let Some(v) = o.roughness {
                p.roughness = v;
            }
            if let Some(v) = o.narrow_pull {
                p.narrow_pull = v;
            }
            if let Some(v) = o.chaikin_iters {
                p.chaikin_iters = v;
            }
        }
        p
    }
}
