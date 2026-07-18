//! Seed-growth: grow N disjoint areas cell by cell, keeping a one-cell gap
//! between areas so that gap cells can later become doorways.

use crate::grid::{Hex, HexGrid};
use crate::tags::{LayoutTag, ShapeTag, SizeTag, Tags};
use rand::Rng;
use std::collections::HashMap;

/// Numeric parameters resolved from tags for the growth step.
#[derive(Clone, Debug)]
pub struct GrowthParams {
    /// Target cell count for each area (first entry is the hub when present).
    pub sizes: Vec<usize>,
    /// Connectedness preference: candidate weight is `c^gamma` where `c` is
    /// the number of neighbours already in the area. High = rounder blobs,
    /// negative = narrow tendrils.
    pub gamma: f64,
    /// Chaotic shape overrides gamma: prefer cells with exactly 2 connections.
    pub chaotic: bool,
}

pub fn resolve<R: Rng>(tags: &Tags, rng: &mut R) -> GrowthParams {
    let count = match tags.size {
        Some(SizeTag::Small) => rng.random_range(2..=3),
        Some(SizeTag::Medium) => rng.random_range(3..=8),
        Some(SizeTag::Large) => rng.random_range(9..=19),
        None => match rng.random_range(0..20) {
            0 => 2,
            19 => 20,
            _ => rng.random_range(3..=12),
        },
    };

    let gamma = match tags.shape {
        Some(ShapeTag::Cavities) => 6.0,
        Some(ShapeTag::Coral) => -rng.random_range(0.5..2.0),
        _ => rng.random_range(0.5..3.0),
    };
    let chaotic = tags.shape == Some(ShapeTag::Chaotic);

    let sizes: Vec<usize> = match tags.layout {
        Some(LayoutTag::Hub) => {
            let mut v = vec![rng.random_range(60..=79)];
            for _ in 1..count.max(2) {
                v.push(rng.random_range(8..=13));
            }
            v
        }
        Some(LayoutTag::Chamber) => {
            let base = rng.random_range(11..=14) as i64;
            (0..count)
                .map(|_| (base + rng.random_range(-2..=2)).max(5) as usize)
                .collect()
        }
        Some(LayoutTag::Burrow) => (0..count)
            .map(|_| {
                let m = (rng.random::<f64>() + rng.random::<f64>() + rng.random::<f64>()) / 3.0;
                (10.0 + 80.0 * m.powi(3)) as usize
            })
            .collect(),
        None => {
            let base = rng.random_range(10..=20) as i64;
            (0..count)
                .map(|_| (base + rng.random_range(-6..=6)).max(5) as usize)
                .collect()
        }
    };

    GrowthParams { sizes, gamma, chaotic }
}

/// Board radius sized so the areas have room to grow plus buffer gaps.
pub fn grid_radius(params: &GrowthParams) -> i32 {
    let total: usize = params.sizes.iter().sum();
    let needed = (total * 3) as i32;
    let mut r = 4;
    while 3 * r * r + 3 * r + 1 < needed {
        r += 1;
    }
    (r + 2).min(40)
}

/// The grown areas. `cells[i]` lists area i's cells in growth order.
pub struct Areas {
    pub cells: Vec<Vec<Hex>>,
    owner: HashMap<Hex, usize>,
}

impl Areas {
    pub fn count(&self) -> usize {
        self.cells.len()
    }

    pub fn owner_of(&self, h: Hex) -> Option<usize> {
        self.owner.get(&h).copied()
    }

    /// Free the given cells of `area` (used by corridor shrinking).
    pub fn remove_from_area(&mut self, area: usize, remove: &[Hex]) {
        for c in remove {
            self.owner.remove(c);
        }
        self.cells[area].retain(|c| !remove.contains(c));
    }
}

/// Areas smaller than this are discarded as failed growths.
const MIN_AREA: usize = 4;

pub fn grow_areas<R: Rng>(grid: &HexGrid, rng: &mut R, params: &GrowthParams) -> Areas {
    let mut owner: HashMap<Hex, usize> = HashMap::new();
    let mut areas: Vec<Vec<Hex>> = Vec::new();

    for &target in &params.sizes {
        let idx = areas.len();

        // Seed anywhere whose whole neighbourhood is free of other areas.
        let valid: Vec<Hex> = grid
            .cells()
            .iter()
            .copied()
            .filter(|&h| {
                !owner.contains_key(&h) && h.neighbors().iter().all(|n| !owner.contains_key(n))
            })
            .collect();
        if valid.is_empty() {
            continue;
        }
        // After the first area, prefer seeds one gap-cell away from an existing
        // area so every area has at least one doorway candidate to a neighbour.
        let owned: Vec<Hex> = grid
            .cells()
            .iter()
            .copied()
            .filter(|h| owner.contains_key(h))
            .collect();
        let near: Vec<Hex> = valid
            .iter()
            .copied()
            .filter(|&h| owned.iter().any(|&o| h.distance(o) == 2))
            .collect();
        // No spot within reach of the existing cave: skip rather than create
        // an unreachable satellite area.
        if near.is_empty() && !areas.is_empty() {
            continue;
        }
        // Start the first area near the centre so the cave grows outward in
        // every direction rather than hugging one side of the board.
        let central: Vec<Hex>;
        let seeds = if !areas.is_empty() {
            &near
        } else {
            central = valid
                .iter()
                .copied()
                .filter(|h| h.distance(Hex::ORIGIN) <= grid.radius / 3)
                .collect();
            if central.is_empty() { &valid } else { &central }
        };
        let seed = seeds[rng.random_range(0..seeds.len())];

        let mut cells = vec![seed];
        owner.insert(seed, idx);

        while cells.len() < target {
            // Candidates: free in-grid cells adjacent to this area whose other
            // neighbours belong to no *other* area (preserves the 1-cell gap).
            let mut cand: Vec<Hex> = Vec::new();
            for &c in &cells {
                for n in c.neighbors() {
                    if grid.contains(n)
                        && !owner.contains_key(&n)
                        && n.neighbors()
                            .iter()
                            .all(|m| owner.get(m).is_none_or(|&o| o == idx))
                    {
                        cand.push(n);
                    }
                }
            }
            cand.sort_unstable();
            cand.dedup();
            if cand.is_empty() {
                break;
            }

            let weights: Vec<f64> = cand
                .iter()
                .map(|&h| {
                    let c = h
                        .neighbors()
                        .iter()
                        .filter(|n| owner.get(n) == Some(&idx))
                        .count();
                    if params.chaotic {
                        if c == 2 { 8.0 } else { 1.0 }
                    } else {
                        (c as f64).powf(params.gamma)
                    }
                })
                .collect();

            let pick = cand[weighted_index(rng, &weights)];
            owner.insert(pick, idx);
            cells.push(pick);
        }

        if cells.len() < MIN_AREA {
            for c in &cells {
                owner.remove(c);
            }
            continue;
        }
        areas.push(cells);
    }

    Areas { cells: areas, owner }
}

pub(crate) fn weighted_index<R: Rng>(rng: &mut R, weights: &[f64]) -> usize {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return rng.random_range(0..weights.len());
    }
    let mut t = rng.random_range(0.0..total);
    for (i, w) in weights.iter().enumerate() {
        t -= w;
        if t < 0.0 {
            return i;
        }
    }
    weights.len() - 1
}
