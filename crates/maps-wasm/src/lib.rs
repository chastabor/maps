//! Thin wasm-bindgen wrapper over maps-core for the web demo.
//!
//! All seeds cross the JS boundary as **strings**: they are u64 values and
//! JavaScript numbers silently corrupt integers above 2^53.

use maps_core::outline::OutlineParams;
use maps_core::tags::Tags;
use maps_core::{GenOptions, GridStyle, Mode, generate_with, random_tags_for, render};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// Master seed (decimal string). Defaults to 0; the UI always sends one.
    seed: Option<String>,
    shape_seed: Option<String>,
    decor_seed: Option<String>,
    name_seed: Option<String>,
    /// "cave" (default) or "forest"/"glade".
    mode: Option<String>,
    /// "hex" (default), "square" or "none".
    grid: Option<String>,
    /// Comma-separated tag list. Absent = roll random tags from the seed;
    /// present but empty = explicitly untagged.
    tags: Option<String>,
    water_level: Option<f64>,
    ruins_level: Option<f64>,
    /// Use this exact map title instead of generating one.
    title: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    svg: String,
    title: String,
    /// The effective tags, as a space-separated display string.
    tags: String,
    seed: String,
    shape_seed: String,
    decor_seed: String,
    name_seed: String,
}

fn err(msg: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&msg.to_string())
}

fn parse_seed(s: &Option<String>, what: &str) -> Result<Option<u64>, JsValue> {
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) => t
            .parse::<u64>()
            .map(Some)
            .map_err(|_| err(format!("{what} must be an unsigned 64-bit integer"))),
    }
}

/// Generate a map. Returns `{ svg, title, tags, seed, shapeSeed, decorSeed,
/// nameSeed }` with the *effective* values, so callers can pin or re-roll
/// individual randomness streams.
#[wasm_bindgen]
pub fn generate(opts: JsValue) -> Result<JsValue, JsValue> {
    let o: Options = serde_wasm_bindgen::from_value(opts).map_err(err)?;
    let seed = parse_seed(&o.seed, "seed")?.unwrap_or(0);
    let mode = match o.mode.as_deref().unwrap_or("cave") {
        "cave" => Mode::Cave,
        "forest" | "glade" => Mode::Forest,
        other => return Err(err(format!("unknown mode: {other} (cave|forest)"))),
    };
    let grid = match o.grid.as_deref().unwrap_or("hex") {
        "hex" => GridStyle::Hex,
        "square" => GridStyle::Square,
        "none" => GridStyle::None,
        other => return Err(err(format!("unknown grid style: {other} (hex|square|none)"))),
    };
    let tags = match &o.tags {
        None => None,
        Some(t) => Some(Tags::parse(t).map_err(err)?),
    };

    let map = generate_with(
        seed,
        &GenOptions {
            mode,
            grid,
            tags,
            outline: OutlineParams::default(),
            water_level: o.water_level.map(|v| v.clamp(0.0, 1.0)),
            ruins_level: o.ruins_level.map(|v| v.clamp(0.0, 1.0)),
            shape_seed: parse_seed(&o.shape_seed, "shapeSeed")?,
            decor_seed: parse_seed(&o.decor_seed, "decorSeed")?,
            name_seed: parse_seed(&o.name_seed, "nameSeed")?,
            title: o.title.clone(),
        },
    );

    let out = Output {
        svg: render::svg(&map),
        title: map.title.clone(),
        tags: map.tags.to_string(),
        seed: seed.to_string(),
        shape_seed: map.shape_seed.to_string(),
        decor_seed: map.decor_seed.to_string(),
        name_seed: map.name_seed.to_string(),
    };
    serde_wasm_bindgen::to_value(&out).map_err(err)
}

/// The tags this master seed would roll if none were supplied — lets the UI
/// preview/edit a seed's random tags. Returned as a comma-separated list
/// ready for the `tags` option ("" when the seed rolls no tags).
#[wasm_bindgen]
pub fn random_tags(seed: &str) -> Result<String, JsValue> {
    let seed = parse_seed(&Some(seed.to_string()), "seed")?.unwrap_or(0);
    let display = random_tags_for(seed).to_string();
    Ok(if display == "(none)" {
        String::new()
    } else {
        display.replace(' ', ",")
    })
}
