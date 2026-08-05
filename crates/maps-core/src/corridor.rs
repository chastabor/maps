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
use crate::geom::Point;
use crate::grid::Hex;
use crate::growth::Areas;
use crate::topology::Connection;

/// One tile of a corridor and the direction(s) it carries the passage.
///
/// Side `k` is the edge from corner `k` to corner `k+1`, neighbour `neighbors()[k]`.
/// `touch_*` is raw room contact — the side faces that room's floor directly. `toward_*`
/// additionally includes sides stepping to a corridor tile strictly nearer that end, so it
/// answers "which way does the passage run" while `touch_*` answers "where does it land".
/// Both are carried because every consumer so far (the overlay, the landing marks, the
/// centerline anchors) needed the raw fact and had to re-derive it when only the fused one
/// was stored.
#[derive(Clone, Debug)]
pub struct TileAxis {
    pub touch_a: [bool; 6],
    pub touch_b: [bool; 6],
    pub toward_a: [bool; 6],
    pub toward_b: [bool; 6],
}

impl TileAxis {
    /// R3: two adjacent sides touching the same room collapse that room's contact to their
    /// shared corner — corner `k+1` of the first such side. `None` when contact is a single
    /// side (or none), which stays an edge landing.
    pub fn collapse(touch: &[bool; 6]) -> Option<usize> {
        (0..6)
            .find(|&k| touch[k] && touch[(k + 1) % 6])
            .map(|k| (k + 1) % 6)
    }
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

/// Where the corridor lands on one room's fitted border: `(tile index, mark)` per touching
/// tile, in tile order. Empty when the side has no room border to land on — an organic
/// partner, or a hall-shaped one (`Areas::room_border`).
pub type Landing = Vec<(usize, Mark)>;

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
    /// The spine's tile path, as indices into `tiles`, `a` end first. Phase 2 clamps each
    /// offset wall segment to its host tile, so the association is kept here rather than
    /// re-located later by point-in-hex tests.
    pub path: Vec<usize>,
    /// Phase 1: the corridor's spine, `a`-landing to `b`-landing. Entry and exit are the
    /// landing anchors (a tile's collapse point, else its touched edge's midpoint, on the
    /// border when the side has one); interior waypoints are the shared-edge midpoints
    /// between consecutive path tiles, so the polyline's interior stays inside the
    /// corridor's tiles (R4). Empty when either side is unreachable through the run.
    pub centerline: Vec<Point>,
}

/// Per-tile raw room contact with `side`.
fn touch(areas: &Areas, side: usize, tile: Hex) -> [bool; 6] {
    let mut t = [false; 6];
    for (k, n) in tile.neighbors().into_iter().enumerate() {
        t[k] = areas.is_room_floor(side, n);
    }
    t
}

/// Graph distance from each tile to the nearest tile with room contact (per `touches`),
/// walking only within the corridor's tiles. `usize::MAX` when unreachable.
fn dist_to(tiles: &[Hex], touches: &[[bool; 6]]) -> Vec<usize> {
    let mut dist = vec![usize::MAX; tiles.len()];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (i, t) in touches.iter().enumerate() {
        if t.iter().any(|&x| x) {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for j in 0..tiles.len() {
            if dist[j] == usize::MAX && tiles[i].distance(tiles[j]) == 1 {
                dist[j] = dist[i] + 1;
                queue.push_back(j);
            }
        }
    }
    dist
}

/// The landing anchor of `tile` on `side`: its collapse corner (R3), else the midpoint of
/// its first touched edge — on the room border when the side has one, on the raw edge
/// otherwise. `None` when the tile does not touch the side at all.
fn landing_anchor(
    areas: &Areas,
    tile: Hex,
    touch: &[bool; 6],
    side: usize,
    s: f64,
) -> Option<Point> {
    let cs = tile.corners(s);
    let raw = if let Some(c) = TileAxis::collapse(touch) {
        cs[c]
    } else {
        let k = (0..6).find(|&k| touch[k])?;
        (
            (cs[k].0 + cs[(k + 1) % 6].0) / 2.0,
            (cs[k].1 + cs[(k + 1) % 6].1) / 2.0,
        )
    };
    Some(match areas.room_border(side) {
        Some(sh) => sh.nearest_on_wall(raw),
        None => raw,
    })
}

/// The landing of `tiles` on `side`'s fitted border: per-tile marks (R3), through the same
/// `nearest_on_wall` locus the anchors use — the marks and the spine ends must never pick
/// different border points for the same contact.
fn attachment(areas: &Areas, tiles: &[Hex], touches: &[[bool; 6]], side: usize, s: f64) -> Landing {
    let Some(sh) = areas.room_border(side) else {
        return Landing::default();
    };
    let mut marks: Landing = Vec::new();
    for (i, (t, touch)) in tiles.iter().zip(touches).enumerate() {
        let cs = t.corners(s);
        if let Some(c) = TileAxis::collapse(touch) {
            marks.push((i, Mark::Point(sh.nearest_on_wall(cs[c]))));
        } else {
            for k in (0..6).filter(|&k| touch[k]) {
                marks.push((
                    i,
                    Mark::Bar(
                        sh.nearest_on_wall(cs[k]),
                        sh.nearest_on_wall(cs[(k + 1) % 6]),
                    ),
                ));
            }
        }
    }
    marks
}

/// The spine's tile path: from the `a`-touching tile nearest `b`, stepping to a neighbour
/// strictly closer to `b` each time. Deterministic: candidates scanned in tile order.
fn spine_path(tiles: &[Hex], da: &[usize], db: &[usize]) -> Vec<usize> {
    let Some(start) = (0..tiles.len())
        .filter(|&i| da[i] == 0)
        .min_by_key(|&i| (db[i], i))
    else {
        return Vec::new();
    };
    if db[start] == usize::MAX {
        return Vec::new();
    }
    let mut path = vec![start];
    let mut cur = start;
    while db[cur] > 0 {
        let Some(next) = (0..tiles.len())
            .filter(|&j| tiles[cur].distance(tiles[j]) == 1 && db[j] < db[cur])
            .min_by_key(|&j| (db[j], j))
        else {
            return Vec::new();
        };
        path.push(next);
        cur = next;
    }
    path
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
            let touch_a: Vec<[bool; 6]> = tiles.iter().map(|&t| touch(areas, c.a, t)).collect();
            let touch_b: Vec<[bool; 6]> = tiles.iter().map(|&t| touch(areas, c.b, t)).collect();
            let da = dist_to(&tiles, &touch_a);
            let db = dist_to(&tiles, &touch_b);
            let axes: Vec<TileAxis> = tiles
                .iter()
                .enumerate()
                .map(|(i, &t)| {
                    let mut toward_a = touch_a[i];
                    let mut toward_b = touch_b[i];
                    for (k, n) in t.neighbors().into_iter().enumerate() {
                        if let Some(j) = tiles.iter().position(|&x| x == n) {
                            if da[j] != usize::MAX && da[i] != usize::MAX && da[j] < da[i] {
                                toward_a[k] = true;
                            }
                            if db[j] != usize::MAX && db[i] != usize::MAX && db[j] < db[i] {
                                toward_b[k] = true;
                            }
                        }
                    }
                    TileAxis {
                        touch_a: touch_a[i],
                        touch_b: touch_b[i],
                        toward_a,
                        toward_b,
                    }
                })
                .collect();
            let path = spine_path(&tiles, &da, &db);
            let centerline = if path.is_empty() {
                Vec::new()
            } else {
                let last = *path.last().unwrap();
                let entry = landing_anchor(areas, tiles[path[0]], &touch_a[path[0]], c.a, s);
                let exit = landing_anchor(areas, tiles[last], &touch_b[last], c.b, s);
                match (entry, exit) {
                    (Some(entry), Some(exit)) => {
                        let mut line = vec![entry];
                        for w in path.windows(2) {
                            let (p, q) = (tiles[w[0]].center(s), tiles[w[1]].center(s));
                            line.push(((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0));
                        }
                        line.push(exit);
                        line
                    }
                    _ => Vec::new(),
                }
            };
            Corridor {
                conn: ci,
                a: c.a,
                b: c.b,
                attach: [
                    attachment(areas, &tiles, &touch_a, c.a, s),
                    attachment(areas, &tiles, &touch_b, c.b, s),
                ],
                path,
                centerline,
                tiles,
                axes,
            }
        })
        .collect()
}
