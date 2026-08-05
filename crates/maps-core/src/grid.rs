//! Axial-coordinate, pointy-top hexagonal grid.

pub(crate) const SQRT3: f64 = 1.732_050_807_568_877_2;
/// A pointy-top hex cell's apothem (flat-edge distance) per unit of size.
pub(crate) const HEX_APOTHEM: f64 = SQRT3 / 2.0;

/// Corner `i` of a pointy-top hex around `center` — angle `60i − 30°`. The one
/// definition of the corner convention: [`Hex::corners`], the outline's raster
/// corners and `RuinShape::HexCell` all delegate here, and the HexCell design
/// depends on them agreeing bit-for-bit (a neck vertex must land exactly on its
/// own cell's corner).
pub(crate) fn hex_corner(center: (f64, f64), i: usize, size: f64) -> (f64, f64) {
    let angle = std::f64::consts::PI / 180.0 * (60.0 * i as f64 - 30.0);
    (center.0 + size * angle.cos(), center.1 + size * angle.sin())
}

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

    /// The two corner indices of the edge facing `neighbors()[k]`.
    ///
    /// **NOT `(k, k + 1)`.** `hex_corner` places corner `i` at `60i - 30` degrees, which
    /// advances counter-clockwise, while [`HEX_DIRS`] advances CLOCKWISE — so edge `k`
    /// (corners `k..k+1`) faces neighbour `(6 - k) % 6`. Measured on a unit hex: edge 1's
    /// midpoint sits at +60 degrees, neighbour 1 at -60.
    ///
    /// Anything that pairs a neighbour with "its" edge must come through here. Assuming
    /// `(k, k + 1)` mirrors the hexagon: the arrows drawn from a corridor tile pointed at the
    /// reflected sides, and it is why a mirrored mouth set still looked plausible.
    pub fn edge_corners(k: usize) -> (usize, usize) {
        let e = (6 - k) % 6;
        (e, (e + 1) % 6)
    }

    pub fn corners(self, size: f64) -> [(f64, f64); 6] {
        let c = self.center(size);
        std::array::from_fn(|i| hex_corner(c, i, size))
    }
}

/// Dense per-cell storage for a board of known extent: O(1) array indexing
/// with no hashing — the hot replacement for `HashMap<Hex, T>` on lookup
/// paths. Cells outside the axial bounding box read as empty (neighbours of
/// rim cells probe there constantly).
///
/// The bounds are per axis because the board is a rectangle: a row's `q` range
/// slides by `−r/2` to keep the left and right edges vertical in pixel space,
/// so `q` spans a good deal wider than `r`. Build one sized to a grid with
/// [`HexGrid::cell_map`] rather than working the bounds out at each call site.
pub struct CellMap<T> {
    q_half: i32,
    r_half: i32,
    width: i32,
    slots: Vec<Option<T>>,
}

impl<T: Copy> CellMap<T> {
    /// Storage for `|q| <= q_half`, `|r| <= r_half`, plus one cell of slack on
    /// every side so a rim cell's neighbours index in bounds instead of
    /// short-circuiting.
    pub fn new(q_half: i32, r_half: i32) -> Self {
        let (q_half, r_half) = (q_half + 1, r_half + 1);
        let width = 2 * q_half + 1;
        let height = 2 * r_half + 1;
        CellMap {
            q_half,
            r_half,
            width,
            slots: vec![None; (width * height) as usize],
        }
    }

    #[inline]
    fn slot(&self, h: Hex) -> Option<usize> {
        if h.q.abs() > self.q_half || h.r.abs() > self.r_half {
            None
        } else {
            Some(((h.r + self.r_half) * self.width + (h.q + self.q_half)) as usize)
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

/// A **rectangular** board, `2·cols+1` columns wide and `2·rows+1` rows tall,
/// centred on the origin.
///
/// Rectangular rather than hexagonal for two reasons. It is the shape a reader
/// expects of a map — the shape of a sheet of paper — and it makes the boundary
/// a pair of independent axis ranges instead of a hex radius, so an
/// out-of-bounds test is a comparison per axis and "how far to the edge" is
/// well defined without a centre to measure from. Geometry fitted to the tiles
/// leans on both (see `plans/tile-first-render.md`).
///
/// The rows alternate between `2·cols+1` and `2·cols` cells, because a row's
/// `q` range slides by `−r/2` to hold the left and right edges vertical in
/// pixel space and only even rows land flush. That half-column ragged edge is
/// inherent to a rectangle on a hex lattice.
pub struct HexGrid {
    /// Half-extent in columns: the widest row spans `q ∈ [−cols, cols]` at `r = 0`.
    pub cols: i32,
    /// Half-extent in rows: `r ∈ [−rows, rows]`.
    pub rows: i32,
    cells: Vec<Hex>,
}

impl HexGrid {
    pub fn rectangle(cols: i32, rows: i32) -> Self {
        let mut cells = Vec::new();
        for r in -rows..=rows {
            for q in Self::q_range(cols, r) {
                cells.push(Hex::new(q, r));
            }
        }
        HexGrid { cols, rows, cells }
    }

    /// The `q` values in row `r`: the integers with `q + r/2 ∈ [−cols, cols]`,
    /// i.e. whose cell centre `x = √3·s·(q + r/2)` falls inside the rectangle.
    #[inline]
    fn q_range(cols: i32, r: i32) -> std::ops::RangeInclusive<i32> {
        // ceil(−cols − r/2) ..= floor(cols − r/2), via floor(±r/2) on integers.
        (-cols - r.div_euclid(2))..=(cols + (-r).div_euclid(2))
    }

    #[inline]
    pub fn contains(&self, h: Hex) -> bool {
        h.r.abs() <= self.rows && Self::q_range(self.cols, h.r).contains(&h.q)
    }

    /// How many cells `h` sits from the nearest edge — `0` on the rim itself,
    /// and `None` off the board. Replaces "distance from the origin" as the
    /// measure of outwardness: on a rectangle the nearest edge is what a
    /// passage heading off the map is aiming for, and unlike a hex radius it
    /// stays meaningful when the board is not square.
    pub fn edge_distance(&self, h: Hex) -> Option<i32> {
        if !self.contains(h) {
            return None;
        }
        let rng = Self::q_range(self.cols, h.r);
        Some(
            (self.rows - h.r)
                .min(h.r + self.rows)
                .min(rng.end() - h.q)
                .min(h.q - rng.start()),
        )
    }

    /// A `CellMap` sized to this board, slack included.
    pub fn cell_map<T: Copy>(&self) -> CellMap<T> {
        // The `−r/2` slide widens the q span by half the row count each way.
        CellMap::new(self.cols + self.rows.div_euclid(2) + 1, self.rows)
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
