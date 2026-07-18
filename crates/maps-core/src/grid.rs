//! Axial-coordinate, pointy-top hexagonal grid.

const SQRT3: f64 = 1.732_050_807_568_877_2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Hex {
    pub q: i32,
    pub r: i32,
}

pub const HEX_DIRS: [Hex; 6] = [
    Hex { q: 1, r: 0 },
    Hex { q: 1, r: -1 },
    Hex { q: 0, r: -1 },
    Hex { q: -1, r: 0 },
    Hex { q: -1, r: 1 },
    Hex { q: 0, r: 1 },
];

impl Hex {
    pub const ORIGIN: Hex = Hex { q: 0, r: 0 };

    pub fn new(q: i32, r: i32) -> Self {
        Hex { q, r }
    }

    pub fn neighbors(self) -> [Hex; 6] {
        HEX_DIRS.map(|d| Hex::new(self.q + d.q, self.r + d.r))
    }

    pub fn distance(self, other: Hex) -> i32 {
        let dq = self.q - other.q;
        let dr = self.r - other.r;
        (dq.abs() + dr.abs() + (dq + dr).abs()) / 2
    }

    /// Pixel center for a pointy-top hex with side length `size`.
    pub fn center(self, size: f64) -> (f64, f64) {
        let x = size * SQRT3 * (self.q as f64 + self.r as f64 / 2.0);
        let y = size * 1.5 * self.r as f64;
        (x, y)
    }

    pub fn corners(self, size: f64) -> [(f64, f64); 6] {
        let (cx, cy) = self.center(size);
        std::array::from_fn(|i| {
            let angle = std::f64::consts::PI / 180.0 * (60.0 * i as f64 - 30.0);
            (cx + size * angle.cos(), cy + size * angle.sin())
        })
    }
}

/// A hexagon-shaped board of cells within `radius` of the origin.
pub struct HexGrid {
    pub radius: i32,
    cells: Vec<Hex>,
}

impl HexGrid {
    pub fn hexagon(radius: i32) -> Self {
        let mut cells = Vec::new();
        for q in -radius..=radius {
            let lo = (-radius).max(-q - radius);
            let hi = radius.min(-q + radius);
            for r in lo..=hi {
                cells.push(Hex::new(q, r));
            }
        }
        HexGrid { radius, cells }
    }

    pub fn contains(&self, h: Hex) -> bool {
        h.distance(Hex::ORIGIN) <= self.radius
    }

    /// All cells in a fixed, deterministic order.
    pub fn cells(&self) -> &[Hex] {
        &self.cells
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}
