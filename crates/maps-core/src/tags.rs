//! Generation tags. Tags act as presets/switches for the numeric parameters
//! used by later generation steps, mirroring the original generator.

use rand::Rng;
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeTag {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutTag {
    Hub,
    Chamber,
    Burrow,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShapeTag {
    Cavities,
    Coral,
    Chaotic,
}

/// How doorways between areas are culled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectTag {
    /// Spanning tree: each area reachable exactly one way, no loops.
    Tree,
    /// Break one edge of every triangle of mutually-adjacent areas.
    Connected,
}

/// Number of openings to the outside.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitTag {
    Sealed,
    Entrance,
    Passage,
    Junction,
}

/// How much water the cave holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuinsTag {
    Ruins,
    Organic,
}

/// Dungeon presence, nested inside ruins: of the areas that ruins turned
/// geometric, `dungeon` regrows a fraction as clean, doored, symmetric rooms
/// instead of weathered ruins; `natural` guarantees none. Untagged means none
/// unless [`GenOptions::dungeon_level`] overrides it. Has no effect where
/// there are no geometric areas (organic maps, `ruins_level` 0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DungeonTag {
    Dungeon,
    Natural,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaterTag {
    Dry,
    Wet,
}

/// Floor tile pattern drawn on ruin-area cells (no effect without ruins).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PatternTag {
    Mosaic,
    Truchet,
    Islamic,
    Plain,
}

/// One optional tag per mutually-exclusive group.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Tags {
    pub size: Option<SizeTag>,
    pub layout: Option<LayoutTag>,
    pub shape: Option<ShapeTag>,
    pub connect: Option<ConnectTag>,
    pub exits: Option<ExitTag>,
    pub water: Option<WaterTag>,
    /// Ruins presence, mirroring the water group: `ruins` regrows about half
    /// the areas as geometry, `organic` guarantees none, untagged allows the
    /// occasional ruin (see `GenOptions::ruins_level` for exact control).
    pub ruins: Option<RuinsTag>,
    /// Dungeon presence within the geometric (ruin) areas: `dungeon` makes a
    /// fraction of them clean, doored, symmetric rooms (see
    /// [`GenOptions::dungeon_level`]); `natural` guarantees none.
    pub dungeon: Option<DungeonTag>,
    /// Ruin floor tile pattern (mosaic/truchet/islamic/plain).
    pub pattern: Option<PatternTag>,
}

impl Tags {
    /// Roll a full set of defaults: every family gets a concrete tag, so
    /// the seed always decides the map's complete character and callers
    /// (`random_tags_for`, the web UI) can report a real per-family default
    /// — the generator is the single source of truth for what an untouched
    /// family means. Weighted to keep the flavour variety of the old
    /// sparse rolls.
    pub fn random<R: Rng>(rng: &mut R) -> Self {
        let size = Some(match rng.random_range(0..100) {
            0..=24 => SizeTag::Small,
            25..=69 => SizeTag::Medium,
            _ => SizeTag::Large,
        });
        let layout = Some(match rng.random_range(0..100) {
            0..=29 => LayoutTag::Hub,
            30..=69 => LayoutTag::Chamber,
            _ => LayoutTag::Burrow,
        });
        let shape = Some(match rng.random_range(0..100) {
            0..=44 => ShapeTag::Cavities,
            45..=74 => ShapeTag::Coral,
            _ => ShapeTag::Chaotic,
        });
        let connect = Some(if rng.random_bool(0.5) {
            ConnectTag::Tree
        } else {
            ConnectTag::Connected
        });
        let exits = Some(match rng.random_range(0..100) {
            0..=9 => ExitTag::Sealed,
            10..=54 => ExitTag::Entrance,
            55..=84 => ExitTag::Passage,
            _ => ExitTag::Junction,
        });
        let water = Some(if rng.random_bool(0.45) {
            WaterTag::Wet
        } else {
            WaterTag::Dry
        });
        let ruins = Some(if rng.random_bool(0.2) {
            RuinsTag::Ruins
        } else {
            RuinsTag::Organic
        });
        let pattern = Some(match rng.random_range(0..100) {
            0..=54 => PatternTag::Plain,
            55..=69 => PatternTag::Mosaic,
            70..=84 => PatternTag::Truchet,
            _ => PatternTag::Islamic,
        });
        // Drawn last so adding this family leaves every other family's rolled
        // value unchanged: only the presence of a dungeon tag is new.
        let dungeon = Some(if rng.random_bool(0.2) {
            DungeonTag::Dungeon
        } else {
            DungeonTag::Natural
        });
        Tags {
            size,
            layout,
            shape,
            connect,
            exits,
            water,
            ruins,
            dungeon,
            pattern,
        }
    }

}

/// Declares every tag family exactly once — field, public family name, and
/// the token ↔ variant mapping — and generates `Tags::parse`,
/// `Display for Tags` and [`tag_families`] from that single table. Adding a
/// family or variant here is the only edit needed for the parser, the
/// display form, the CLI help and the web UI's radio groups.
macro_rules! tag_families {
    ($($field:ident ($name:literal, $ty:ident): $($tok:literal => $var:ident),+ ;)+) => {
        /// Family names with their canonical tokens, in display order — the
        /// single source of truth for every tag list (CLI help, web UI).
        pub fn tag_families() -> &'static [(&'static str, &'static [&'static str])] {
            &[$(($name, &[$($tok),+])),+]
        }

        impl Tags {
            /// Parse a comma- or space-separated tag list, e.g.
            /// `"large,hub,coral"`.
            pub fn parse(s: &str) -> Result<Tags, String> {
                let mut tags = Tags::default();
                for token in s.split([',', ' ']).filter(|t| !t.is_empty()) {
                    match token.to_ascii_lowercase().as_str() {
                        $($($tok => tags.$field = Some($ty::$var),)+)+
                        other => return Err(format!("unknown tag: {other}")),
                    }
                }
                Ok(tags)
            }
        }

        impl fmt::Display for Tags {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut names: Vec<&str> = Vec::new();
                $(
                    if let Some(v) = self.$field {
                        names.push(match v { $($ty::$var => $tok,)+ });
                    }
                )+
                if names.is_empty() {
                    write!(f, "(none)")
                } else {
                    write!(f, "{}", names.join(" "))
                }
            }
        }
    };
}

tag_families! {
    size ("size", SizeTag): "small" => Small, "medium" => Medium, "large" => Large;
    layout ("layout", LayoutTag): "hub" => Hub, "chamber" => Chamber, "burrow" => Burrow;
    shape ("shape", ShapeTag): "cavities" => Cavities, "coral" => Coral, "chaotic" => Chaotic;
    connect ("links", ConnectTag): "tree" => Tree, "connected" => Connected;
    exits ("exits", ExitTag): "sealed" => Sealed, "entrance" => Entrance, "passage" => Passage, "junction" => Junction;
    water ("water", WaterTag): "wet" => Wet, "dry" => Dry;
    ruins ("ruins", RuinsTag): "ruins" => Ruins, "organic" => Organic;
    dungeon ("dungeon", DungeonTag): "dungeon" => Dungeon, "natural" => Natural;
    pattern ("pattern", PatternTag): "mosaic" => Mosaic, "truchet" => Truchet, "islamic" => Islamic, "plain" => Plain;
}
