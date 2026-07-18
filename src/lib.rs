//! maps: generate watabou-style cave maps as SVG, driven by a TOML
//! configuration file. The generation engine lives in the `maps-core` crate,
//! re-exported here as [`core`].

pub use maps_core as core;

pub mod config;
