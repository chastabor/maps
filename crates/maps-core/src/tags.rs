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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaterTag {
    Dry,
    Wet,
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
        Tags {
            size,
            layout,
            shape,
            connect,
            exits,
            water,
            ruins,
        }
    }

    /// Parse a comma- or space-separated tag list, e.g. `"large,hub,coral"`.
    pub fn parse(s: &str) -> Result<Tags, String> {
        let mut tags = Tags::default();
        for token in s.split([',', ' ']).filter(|t| !t.is_empty()) {
            match token.to_ascii_lowercase().as_str() {
                "small" => tags.size = Some(SizeTag::Small),
                "medium" => tags.size = Some(SizeTag::Medium),
                "large" => tags.size = Some(SizeTag::Large),
                "hub" => tags.layout = Some(LayoutTag::Hub),
                "chamber" => tags.layout = Some(LayoutTag::Chamber),
                "burrow" => tags.layout = Some(LayoutTag::Burrow),
                "cavities" => tags.shape = Some(ShapeTag::Cavities),
                "coral" => tags.shape = Some(ShapeTag::Coral),
                "chaotic" => tags.shape = Some(ShapeTag::Chaotic),
                "tree" => tags.connect = Some(ConnectTag::Tree),
                "connected" => tags.connect = Some(ConnectTag::Connected),
                "sealed" => tags.exits = Some(ExitTag::Sealed),
                "entrance" => tags.exits = Some(ExitTag::Entrance),
                "passage" => tags.exits = Some(ExitTag::Passage),
                "junction" => tags.exits = Some(ExitTag::Junction),
                "dry" => tags.water = Some(WaterTag::Dry),
                "wet" => tags.water = Some(WaterTag::Wet),
                "ruins" => tags.ruins = Some(RuinsTag::Ruins),
                "organic" => tags.ruins = Some(RuinsTag::Organic),
                other => return Err(format!("unknown tag: {other}")),
            }
        }
        Ok(tags)
    }
}

impl fmt::Display for Tags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&str> = Vec::new();
        if let Some(s) = self.size {
            names.push(match s {
                SizeTag::Small => "small",
                SizeTag::Medium => "medium",
                SizeTag::Large => "large",
            });
        }
        if let Some(l) = self.layout {
            names.push(match l {
                LayoutTag::Hub => "hub",
                LayoutTag::Chamber => "chamber",
                LayoutTag::Burrow => "burrow",
            });
        }
        if let Some(sh) = self.shape {
            names.push(match sh {
                ShapeTag::Cavities => "cavities",
                ShapeTag::Coral => "coral",
                ShapeTag::Chaotic => "chaotic",
            });
        }
        if let Some(c) = self.connect {
            names.push(match c {
                ConnectTag::Tree => "tree",
                ConnectTag::Connected => "connected",
            });
        }
        if let Some(e) = self.exits {
            names.push(match e {
                ExitTag::Sealed => "sealed",
                ExitTag::Entrance => "entrance",
                ExitTag::Passage => "passage",
                ExitTag::Junction => "junction",
            });
        }
        if let Some(w) = self.water {
            names.push(match w {
                WaterTag::Dry => "dry",
                WaterTag::Wet => "wet",
            });
        }
        if let Some(r) = self.ruins {
            names.push(match r {
                RuinsTag::Ruins => "ruins",
                RuinsTag::Organic => "organic",
            });
        }
        if names.is_empty() {
            write!(f, "(none)")
        } else {
            write!(f, "{}", names.join(" "))
        }
    }
}
