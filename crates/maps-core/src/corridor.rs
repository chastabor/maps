//! Tile-first corridor model — `plans/tile-corridor-render.md`, phase 0.
//!
//! A [`Corridor`] is the render-side realization of one
//! [`Connection`] — the same tiles, but with the *direction*
//! facts the render needs made explicit — which way each tile carries the passage, and
//! where the passage lands on each room's fitted border. The reference for every rule here
//! is `samples/grow-tile-render.png`, the hand-annotated growth view: the arrows in that
//! image are exactly what [`corridors`] derives.
//!
//! Derived from the current `areas` at the point of use and never stored across a mutation,
//! per the join-kind doctrine (`outline::JoinKind`): ownership and adjacency are
//! time-varying, so a snapshot would go stale.
//!
//! Phase 0 is data + the growth-view overlay only. Nothing in the finished render reads
//! this yet, and the growth view is excluded from every content digest by design.

use crate::AreaKind;
use crate::grid::Hex;
use crate::growth::Areas;
use crate::topology::Connection;

type Point = (f64, f64);

/// One tile of a corridor and the direction(s) it carries the passage.
///
/// `toward_a[k]` / `toward_b[k]` say whether side `k` (edge from corner `k` to corner
/// `k+1`, neighbour `neighbors()[k]`) faces the `a` end or the `b` end — either that
/// side's room floor directly, or a corridor tile strictly nearer to it.
#[derive(Clone, Debug)]
pub struct TileAxis {
    pub tile: Hex,
    pub toward_a: [bool; 6],
    pub toward_b: [bool; 6],
    /// R1: some `a`-facing side is exactly opposite a `b`-facing side, so the passage runs
    /// straight through across the flats. Otherwise the path bends inside this tile (R2).
    pub through: bool,
}

/// One mark of a corridor's landing on a room's fitted border.
///
/// Granularity is **per tile**, read off the annotated reference: each tile that touches the
/// room contributes its own mark, and only a tile touching on two or more adjacent sides
/// collapses its own arrows to the shared corner (R3). A corridor-wide collapse — one point
/// for the whole side because a single tile had multi-side contact — drew every arrow of a
/// two-lane corridor into one fan, which is not what the annotation shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A tile's multi-side contact, collapsed to the shared corner projected on the border.
    Point(Point),
    /// One shared edge, projected onto the border (both corners).
    Bar(Point, Point),
}

/// Where the corridor lands on one room's fitted border: every tile's marks, in tile order.
/// Empty when the side has no fitted shape (an organic partner keeps its free-gap trace).
#[derive(Clone, Debug, Default)]
pub struct Landing {
    pub marks: Vec<Mark>,
}

impl Landing {
    /// The collapse point a tile's arrows aim at, if this landing has one.
    pub fn point(&self) -> Option<Point> {
        self.marks.iter().find_map(|m| match m {
            Mark::Point(p) => Some(*p),
            _ => None,
        })
    }
}

/// One connection, realized as tiles + directions + landings.
#[derive(Clone, Debug)]
pub struct Corridor {
    /// Index into `topology.connections`.
    pub conn: usize,
    pub a: usize,
    pub b: usize,
    pub tiles: Vec<Hex>,
    /// Parallel to `tiles`.
    pub axes: Vec<TileAxis>,
    /// Landing on `a`'s border, then on `b`'s.
    pub attach: [Landing; 2],
}

/// Room floor of `side`: owned by it and not join floor (join floor is the corridor's own).
fn room_floor(areas: &Areas, side: usize, h: Hex) -> bool {
    areas.owner_of(h) == Some(side) && !areas.is_join(h)
}

/// Graph distance from each tile to the nearest tile adjacent to `side`'s room floor,
/// walking only within the corridor's tiles. `usize::MAX` when unreachable.
fn dist_to(areas: &Areas, tiles: &[Hex], side: usize) -> Vec<usize> {
    let mut dist = vec![usize::MAX; tiles.len()];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (i, t) in tiles.iter().enumerate() {
        if t.neighbors().iter().any(|n| room_floor(areas, side, *n)) {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for (j, t) in tiles.iter().enumerate() {
            if dist[j] == usize::MAX && tiles[i].distance(*t) == 1 {
                dist[j] = dist[i] + 1;
                queue.push_back(j);
            }
        }
    }
    dist
}

/// The landing of `tiles` on `side`'s fitted border: per-tile marks (R3).
fn attachment(areas: &Areas, tiles: &[Hex], side: usize, s: f64) -> Landing {
    let Some(sh) = areas.shape(side) else {
        return Landing::default();
    };
    let mut marks: Vec<Mark> = Vec::new();
    for t in tiles {
        let cs = t.corners(s);
        let touching: Vec<usize> = t
            .neighbors()
            .into_iter()
            .enumerate()
            .filter(|&(_, n)| room_floor(areas, side, n))
            .map(|(k, _)| k)
            .collect();
        // Two adjacent touching sides collapse THIS tile's contact to their shared corner.
        let collapse = touching
            .iter()
            .find(|&&k| touching.contains(&((k + 1) % 6)))
            .map(|&k| cs[(k + 1) % 6]);
        if let Some(c) = collapse {
            marks.push(Mark::Point(sh.project(c)));
        } else {
            for &k in &touching {
                marks.push(Mark::Bar(sh.project(cs[k]), sh.project(cs[(k + 1) % 6])));
            }
        }
    }
    Landing { marks }
}

/// Derive every dungeon-touching connection's [`Corridor`]. Pure; reads `areas` as it is
/// NOW — call it where the result is consumed.
pub fn corridors(areas: &Areas, connections: &[Connection], s: f64) -> Vec<Corridor> {
    connections
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            areas.kind(c.a) == AreaKind::Dungeon || areas.kind(c.b) == AreaKind::Dungeon
        })
        .map(|(ci, c)| {
            let tiles: Vec<Hex> = c.along.clone();
            let da = dist_to(areas, &tiles, c.a);
            let db = dist_to(areas, &tiles, c.b);
            let axes: Vec<TileAxis> = tiles
                .iter()
                .enumerate()
                .map(|(i, &t)| {
                    let mut toward_a = [false; 6];
                    let mut toward_b = [false; 6];
                    for (k, n) in t.neighbors().into_iter().enumerate() {
                        if room_floor(areas, c.a, n) {
                            toward_a[k] = true;
                        }
                        if room_floor(areas, c.b, n) {
                            toward_b[k] = true;
                        }
                        if let Some(j) = tiles.iter().position(|&x| x == n) {
                            if da[j] != usize::MAX && da[i] != usize::MAX && da[j] < da[i] {
                                toward_a[k] = true;
                            }
                            if db[j] != usize::MAX && db[i] != usize::MAX && db[j] < db[i] {
                                toward_b[k] = true;
                            }
                        }
                    }
                    let through = (0..6).any(|k| toward_a[k] && toward_b[(k + 3) % 6]);
                    TileAxis {
                        tile: t,
                        toward_a,
                        toward_b,
                        through,
                    }
                })
                .collect();
            Corridor {
                conn: ci,
                a: c.a,
                b: c.b,
                attach: [
                    attachment(areas, &tiles, c.a, s),
                    attachment(areas, &tiles, c.b, s),
                ],
                tiles,
                axes,
            }
        })
        .collect()
}
