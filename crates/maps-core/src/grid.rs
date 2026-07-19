//! Axial-coordinate, pointy-top hexagonal grid.

pub(crate) const SQRT3: f64 = 1.732_050_807_568_877_2;

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

    /// The hex containing a pixel point (inverse of `center`).
    pub fn at(p: (f64, f64), size: f64) -> Hex {
        let qf = (SQRT3 / 3.0 * p.0 - p.1 / 3.0) / size;
        let rf = (2.0 / 3.0 * p.1) / size;
        // Cube rounding.
        let sf = -qf - rf;
        let (mut q, mut r, s) = (qf.round(), rf.round(), sf.round());
        let (dq, dr, ds) = ((q - qf).abs(), (r - rf).abs(), (s - sf).abs());
        if dq > dr && dq > ds {
            q = -r - s;
        } else if dr > ds {
            r = -q - s;
        }
        Hex::new(q as i32, r as i32)
    }

    pub fn corners(self, size: f64) -> [(f64, f64); 6] {
        let (cx, cy) = self.center(size);
        std::array::from_fn(|i| {
            let angle = std::f64::consts::PI / 180.0 * (60.0 * i as f64 - 30.0);
            (cx + size * angle.cos(), cy + size * angle.sin())
        })
    }
}

/// Dense per-cell storage for a board of known radius: O(1) array indexing
/// with no hashing — the hot replacement for `HashMap<Hex, T>` on lookup
/// paths. Cells outside the axial bounding box read as empty (neighbours of
/// rim cells probe there constantly).
pub struct CellMap<T> {
    radius: i32,
    width: i32,
    slots: Vec<Option<T>>,
}

impl<T: Copy> CellMap<T> {
    pub fn new(radius: i32) -> Self {
        let width = 2 * radius + 1;
        CellMap {
            radius,
            width,
            slots: vec![None; (width * width) as usize],
        }
    }

    #[inline]
    fn slot(&self, h: Hex) -> Option<usize> {
        if h.q.abs() > self.radius || h.r.abs() > self.radius {
            None
        } else {
            Some(((h.q + self.radius) * self.width + (h.r + self.radius)) as usize)
        }
    }

    #[inline]
    pub fn get(&self, h: Hex) -> Option<T> {
        self.slot(h).and_then(|i| self.slots[i])
    }

    #[inline]
    pub fn contains(&self, h: Hex) -> bool {
        self.slot(h).is_some_and(|i| self.slots[i].is_some())
    }

    pub fn insert(&mut self, h: Hex, v: T) {
        if let Some(i) = self.slot(h) {
            self.slots[i] = Some(v);
        }
    }

    pub fn remove(&mut self, h: Hex) {
        if let Some(i) = self.slot(h) {
            self.slots[i] = None;
        }
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
