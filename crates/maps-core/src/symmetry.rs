//! Symmetry primitives for the growth engine: the exact hex-lattice
//! transforms and a per-map symmetry plan. The engine (`growth`) grows a
//! generator dungeon room and its sibling copies in lockstep, so a wing comes
//! out exactly symmetric and self-sizing.
//!
//! All symmetry is expressed as **sibling orbits**: a generator plus its
//! copies under a small set of transforms about a shared centre. Bilateral,
//! 180°, translated ("identical") and radial (3-/6-fold) wings are all just
//! different transform sets. The hex lattice only has exact rotational
//! symmetry of order 2, 3 and 6, so those are the only rotations offered.

use crate::grid::Hex;
use crate::outline::Point;
use rand::Rng;

const SIN60: f64 = 0.866_025_403_784_438_6; // √3/2

/// A lattice reflection/rotation about the origin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sym {
    FlipX,
    FlipY,
    Rot180,
    Rot60,
    Rot120,
}

impl Sym {
    fn hex_once(self, q: i32, r: i32) -> (i32, i32) {
        match self {
            Sym::FlipX => (-q - r, r),
            Sym::FlipY => (q + r, -r),
            Sym::Rot180 => (-q, -r),
            Sym::Rot60 => (-r, q + r),
            Sym::Rot120 => (-q - r, q),
        }
    }

    fn point_once(self, dx: f64, dy: f64) -> (f64, f64) {
        match self {
            Sym::FlipX => (-dx, dy),
            Sym::FlipY => (dx, -dy),
            Sym::Rot180 => (-dx, -dy),
            Sym::Rot60 => (0.5 * dx - SIN60 * dy, SIN60 * dx + 0.5 * dy),
            Sym::Rot120 => (-0.5 * dx - SIN60 * dy, SIN60 * dx - 0.5 * dy),
        }
    }

    /// Whether this map keeps an axis-aligned rectangle axis-aligned (only
    /// then can a rect room's wall shape be mirrored without rotating it).
    pub fn keeps_rect(self) -> bool {
        matches!(self, Sym::FlipX | Sym::FlipY | Sym::Rot180)
    }
}

/// One sibling's transform, relative to the orbit's generator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Xform {
    /// The generator itself.
    Identity,
    /// Reflect/rotate `times` steps about the orbit centre.
    About { sym: Sym, times: u32 },
    /// A translated copy (an "identical" pair), same orientation.
    Translate { dq: i32, dr: i32 },
}

impl Xform {
    /// Map a cell of the generator to the corresponding sibling cell.
    pub fn cell(self, centre: Hex, c: Hex) -> Hex {
        match self {
            Xform::Identity => c,
            Xform::Translate { dq, dr } => Hex::new(c.q + dq, c.r + dr),
            Xform::About { sym, times } => {
                let (mut q, mut r) = (c.q - centre.q, c.r - centre.r);
                for _ in 0..times {
                    (q, r) = sym.hex_once(q, r);
                }
                Hex::new(centre.q + q, centre.r + r)
            }
        }
    }

    /// Map a pixel point (a room's shape centre) the same way.
    pub fn point(self, centre: Point, p: Point) -> Point {
        match self {
            Xform::Identity => p,
            // Translation in pixel space uses the cell-offset's pixel image.
            Xform::Translate { dq, dr } => {
                let d = Hex::new(dq, dr).center(1.0);
                // center(1.0) of an offset is linear, so scale by hex size at
                // the call site is unnecessary — callers pass hex-sized points,
                // and the offset must match; use the offset in the same units.
                (p.0 + d.0, p.1 + d.1)
            }
            Xform::About { sym, times } => {
                let (mut dx, mut dy) = (p.0 - centre.0, p.1 - centre.1);
                for _ in 0..times {
                    (dx, dy) = sym.point_once(dx, dy);
                }
                (centre.0 + dx, centre.1 + dy)
            }
        }
    }

    /// Whether this transform preserves an axis-aligned rectangle's shape.
    pub fn keeps_rect(self) -> bool {
        match self {
            Xform::Identity | Xform::Translate { .. } => true,
            Xform::About { sym, .. } => sym.keeps_rect(),
        }
    }
}

/// A per-map symmetry plan: the sibling transforms of one orbit (`xforms[0]`
/// is always `Identity`, the generator) and how many generator rooms share
/// this symmetry.
#[derive(Clone, Debug)]
pub struct SymPlan {
    pub xforms: Vec<Xform>,
    pub generators: usize,
}

/// Choose a symmetry plan for the map (or `None` for an asymmetric map).
/// Bilateral / 180° / translated take 1–3 generator rooms; radial takes one
/// generator with a 3- or 6-fold orbit.
pub fn choose<R: Rng>(rng: &mut R) -> Option<SymPlan> {
    let gens = |rng: &mut R| rng.random_range(1..=3);
    Some(match rng.random_range(0..100) {
        // ~30% asymmetric.
        0..30 => return None,
        30..58 => {
            let sym = if rng.random_bool(0.5) { Sym::FlipX } else { Sym::FlipY };
            SymPlan { xforms: vec![Xform::Identity, Xform::About { sym, times: 1 }], generators: gens(rng) }
        }
        58..80 => SymPlan {
            xforms: vec![Xform::Identity, Xform::About { sym: Sym::Rot180, times: 1 }],
            generators: gens(rng),
        },
        80..90 => {
            // Translated ("identical") pair: a random 3–6 cell offset.
            let dq = rng.random_range(-5..=5);
            let dr = rng.random_range(3..=6) * if rng.random_bool(0.5) { 1 } else { -1 };
            SymPlan { xforms: vec![Xform::Identity, Xform::Translate { dq, dr }], generators: gens(rng) }
        }
        90..97 => SymPlan {
            xforms: vec![
                Xform::Identity,
                Xform::About { sym: Sym::Rot120, times: 1 },
                Xform::About { sym: Sym::Rot120, times: 2 },
            ],
            generators: 1,
        },
        _ => SymPlan {
            xforms: (0..6)
                .map(|k| {
                    if k == 0 {
                        Xform::Identity
                    } else {
                        Xform::About { sym: Sym::Rot60, times: k }
                    }
                })
                .collect(),
            generators: 1,
        },
    })
}
