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
use crate::clip::{Span, merge_groups};
use crate::geom::{Point, farthest_pair};
use crate::grid::Hex;
use crate::growth::Areas;
use crate::ruins::RuinShape;
use crate::topology::Connection;

/// One tile of a corridor and the direction(s) it carries the passage.
///
/// Side `k` means neighbour `neighbors()[k]`; its edge's corners come from
/// [`Hex::edge_corners`] (edge `k` faces neighbour `(6-k)%6`, NOT `k`).
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
    /// R3: two adjacent sides touching the same room collapse that room's contact to the
    /// corner their two edges share. `None` when contact is a single side (or none), which
    /// stays an edge landing.
    pub fn collapse(touch: &[bool; 6]) -> Option<usize> {
        // Neighbour `k` faces edge `(6-k)%6` and neighbour `k+1` faces `(5-k)%6`; those two
        // edges share corner `(6-k)%6`, which is `edge_corners(k).0`. Deriving it as `k+1`
        // named a corner on the mirrored side of the tile.
        (0..6)
            .find(|&k| touch[k] && touch[(k + 1) % 6])
            .map(|k| Hex::edge_corners(k).0)
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

/// One contiguous stretch of a room's border a corridor's contact covers, and the door that
/// closes it — phase 3's **ring gap**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// The stretch in the room's own wall parameter ([`RuinShape::wall_param`]), wrap-safe.
    ///
    /// This is the space the gap has to be expressed in, not point space. Projecting a chord's
    /// two ends and joining them straight cuts a rect corner clean off whenever the attachment
    /// straddles one — the 2D<->3D acceptance case — because the segment between two points on
    /// perpendicular walls does not follow the wall. A parameter interval follows the border by
    /// construction, corner or no corner, and [`crate::clip::merge`] already makes the wrap safe.
    pub span: Span,
    /// The contact **measured in tile space** (unprojected): the two farthest-apart vertices of
    /// the touched tile edges.
    ///
    /// The width the passage presents to the room before the border has any say, and a lattice
    /// constant: `s` across one shared edge, `√3·s` across two, `2s` across three (12 / 20.78 /
    /// 24px at `s = 12`). Kept because [`Self::span`] is not that measure — a span is arc length on
    /// the border, which stretches with whatever shape the contact happens to land on.
    ///
    /// **Not where the door goes.** An earlier draft had the wall step OUT to this chord to carry
    /// the leaf — the "porch". A rect's border sits an apothem inside its outer tile column, so a
    /// leaf out here floats an apothem clear of the wall it is meant to close, which is exactly
    /// what the user rejected on sight (`plans/wip/phase3-leaf-on-wall.png`). The door is
    /// [`Self::leaf`], on the wall. Nothing load-bearing reads this today — the width rule reads
    /// [`Self::edges`] — it is the *measurement* field, what the overlay draws dashed and the
    /// invariant tests and `examples/corridor_gaps.rs` measure the lattice classes off.
    pub chord: (Point, Point),
    /// How many tile edges the contact spans — the input to the door-width rule, [`Self::opening`].
    /// At least 1: a gap exists only where some edge touched.
    pub edges: usize,
}

/// The **door opening** for a contact of `span` covering `edges` tile edges: one apothem per
/// PAIR of touched edges, centred on the contact and never wider than it.
///
/// The user's policy, chosen over opening the whole contact. Measured against that alternative
/// in `examples/gap_policy.rs`: full contact draws 5.8x less wall across passage floor but
/// removes 60% more wall, and a small circular room loses a third of its ring. Corner loss is
/// identical either way, so it did not enter the decision.
///
/// A free function over `(span, edges)` rather than only a [`Gap`] method, because phase 3c's
/// cross-corridor merge produces exactly such a pair (the union span, the summed edges) with no
/// `Gap` to call through — and a policy reachable only via one struct would get re-derived there.
pub fn door_opening(span: Span, edges: usize, per: f64, s: f64) -> Span {
    let want = (crate::grid::HEX_APOTHEM * s * edges.div_ceil(2) as f64).min(span.len);
    Span {
        from: (span.from + (span.len - want) / 2.0).rem_euclid(per),
        len: want,
    }
}

/// The **leaf** closing `opening`: a straight segment joining the two points where the wall
/// stops.
///
/// One rule for every shape, with no clamping and no arc:
/// - a circle's door is a **flat** leaf across its opening, not a curved one;
/// - an opening that turns a rect corner gets one **angled** leaf across the corner, never an
///   L. That is what retires `doorway::clamp_opening`, whose whole job was sliding an opening
///   off a corner so that a straight bar could close it — take the leaf as the opening's chord
///   and a corner opening closes itself.
pub fn door_leaf(sh: &RuinShape, opening: Span, per: f64) -> (Point, Point) {
    (sh.wall_point(opening.from), sh.wall_point(opening.end(per)))
}

impl Gap {
    /// [`door_opening`] for this gap's own contact.
    pub fn opening(&self, per: f64, s: f64) -> Span {
        door_opening(self.span, self.edges, per, s)
    }

    /// [`door_leaf`] for this gap's own opening.
    pub fn leaf(&self, sh: &RuinShape, per: f64, s: f64) -> (Point, Point) {
        door_leaf(sh, self.opening(per, s), per)
    }
}

/// Where the corridor lands on one room's fitted border.
///
/// Two views of one contact, because phase 3's consumers need different ones: the arrows are
/// per tile and live on the border, while the gap to cut is an interval and the leaf stands on
/// the tile boundary. Empty when the side has no room border to land on — an organic partner,
/// or a hall-shaped one (`Areas::room_border`).
#[derive(Clone, Debug, Default)]
pub struct Landing {
    /// Per touching tile, in tile order — the phase-0 deliverable, verified arrow-for-arrow
    /// against the annotated reference, and projected onto the border for that reason.
    pub marks: Vec<(usize, Mark)>,
    /// The contact as border stretches, merged. Normally one; two means the corridor touches
    /// this room at two separated places, which the merge keeps apart rather than bridging into
    /// a gap that spans intact wall.
    pub gaps: Vec<Gap>,
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
    /// Phase 1: the corridor's spine, `a`-landing to `b`-landing. Entry and exit are the
    /// landing anchors (a tile's collapse point, else its touched edge's midpoint, on the
    /// border when the side has one); interior waypoints are the shared-edge midpoints
    /// between consecutive path tiles, so the polyline's interior stays inside the
    /// corridor's tiles (R4). Empty when either side is unreachable through the run.
    pub centerline: Vec<Point>,
}

impl Corridor {
    /// Each landing with the area it lands on: `attach[0]` with `a`, `attach[1]` with `b` — the
    /// zip every consumer wrote out by hand (the overlay, the tests, both measurement examples).
    pub fn landings(&self) -> impl Iterator<Item = (&Landing, usize)> {
        self.attach.iter().zip([self.a, self.b])
    }

    /// Phase 2: the corridor's two walls, `[low, high]` across the passage, as straight
    /// segments on **lattice lines through the tiles the corridor occupies** — a dungeon
    /// rectangle laid over the joining tiles, not an offset from the spine.
    ///
    /// The lattice, not the spine, fixes where a wall may sit. Offsetting the centerline put
    /// walls wherever the arithmetic landed, off the tile edges entirely; following the tile
    /// boundary instead chevrons, because across travel a pointy-top hex presents two edges
    /// meeting at a vertex. A pointy-top lattice offers a line every 30 degrees, because it has
    /// three axis families 60 degrees apart and each contributes two perpendicular side
    /// directions:
    ///
    /// - `30 / 90 / 150` — lines carrying actual hex EDGES         -> pad one apothem
    /// - `0 / 60 / 120`  — lines through the shoulder VERTEX pairs -> pad half a hex
    ///
    /// (The 0-degree case is the familiar dungeon rect: vertical sides on the tiles' own edge
    /// lines, top and bottom at the shoulders with the row peaks left outside, per R4.)
    ///
    /// **Length comes from the two room borders, never from the tiles.** A corridor whose tiles
    /// sit side by side *across* the wall has zero tile extent *along* it — measured on
    /// 5D<->6D, where both tiles project to -36.0 — so deriving the span from the tiles
    /// collapsed the wall to a point. Each wall runs from one room's border crossing to the
    /// other's, which is also what makes the caps structural. (A side with no border — organic
    /// or hall-shaped — is the one exception: that end takes the tile bound.)
    ///
    /// **Placement is a search, not a formula.** Candidates are the lattice lines bounding the
    /// tiles; they are tried outermost-inward and the first one that reaches both borders
    /// without crossing unclaimed ground wins. See the walk below for why each condition is
    /// there and which cheaper rules it replaced.
    ///
    /// **The caps are the exact crossing points, so wall and border meet at a sharp corner.**
    /// Nothing here chamfers: a cut corner would leave no square jamb for a door to sit in.
    pub fn walls(&self, areas: &Areas, s: f64) -> [Vec<Point>; 2] {
        let c = &self.centerline;
        if c.len() < 2 || self.tiles.is_empty() {
            return [Vec::new(), Vec::new()];
        }
        // No CLAIMED floor, no corridor to wall. `topology::build` reserves join floor for
        // dungeon-to-dungeon connections only — an organic area has no fitted wall to build, so
        // its connections stay free cells the outline traces organically, and walling them would
        // put dungeon masonry across a cave passage. Such a connection still contributes its
        // dungeon-side landing (`attach`) for the door; it just has no walls of its own.
        if !self.tiles.iter().any(|&t| areas.is_join(t)) {
            return [Vec::new(), Vec::new()];
        }
        let ap = crate::grid::HEX_APOTHEM * s;
        // Travel comes from the AXES — the arrows — not from the spine's endpoints. A through
        // tile pairs side `k` toward one room with side `k+3` toward the other (R1), so the
        // pairing names the passage's lattice direction exactly; the spine's end-to-end vector
        // only approximates it, and on a two-tile corridor it tilts off axis.
        let travel = {
            let mut votes = [0usize; 3];
            for ax in &self.axes {
                for k in 0..6 {
                    if ax.toward_a[k] && ax.toward_b[(k + 3) % 6] {
                        votes[k % 3] += 1;
                    }
                }
            }
            // Family `k` travels at `-60k` degrees: `grid::HEX_DIRS` runs CLOCKWISE, so
            // neighbour `k` lies at `-60k`. (`+60k` swapped the two diagonal families with each
            // other, walling 100<->4D along 1D<->7D's axis and vice versa.)
            let dir = |k: usize| {
                let a = -(k as f64) * std::f64::consts::PI / 3.0;
                (a.cos(), a.sin())
            };
            let top = votes.iter().copied().max().unwrap_or(0);
            let winners: Vec<usize> = (0..3).filter(|&k| votes[k] == top).collect();
            match (top, winners.as_slice()) {
                (_, [k]) if top > 0 => dir(*k),
                // A TIE means the corridor bends: its tiles pair on two different axes. Take
                // the AVERAGE of the two travel directions by summing the unit vectors, which
                // bisects them. The bisector sits 30 degrees off each, so it lands on a
                // tile-EDGE line rather than a shoulder line and pads by a full apothem —
                // which is also the WIDEST lane the tiles contain. Measured on 5D<->6D, the tie
                // this rule exists for: 30.0px on family 0, 38.8px on family 1, 12.0px on
                // family 2, and 41.6px on the averaged edge line.
                (_, [i, j]) if top > 0 => {
                    let (a, b) = (dir(*i), dir(*j));
                    (a.0 + b.0, a.1 + b.1)
                }
                // No opposite pairing, or all three tied: fall back to the spine's own
                // direction and let the snap below pick the nearest lattice line.
                _ => (c[c.len() - 1].0 - c[0].0, c[c.len() - 1].1 - c[0].1),
            }
        };
        let step = std::f64::consts::PI / 6.0;
        let k = (travel.1.atan2(travel.0) / step).round() as i64;
        let ang = k as f64 * step;
        let (u, n) = ((ang.cos(), ang.sin()), (-ang.sin(), ang.cos()));
        // The two lattice pads, one parity rule stated once. Across the wall (`pad`) and along
        // it (`pad_along`) are exact complements, because a wall sits `pad` off the tile centres
        // where the tile is only `pad_along` wide:
        //
        // - `u` a neighbour direction (even `k`): wall on a shoulder line, tile at full
        //   across-flats width -> `pad = s/2` across, `pad_along = apothem`;
        // - `u` a vertex direction (odd `k`): wall ON a tile edge, tile spans just that edge
        //   -> `pad = apothem` across, `pad_along = s/2`.
        //
        // Using the tile's MAXIMAL extent along the wall instead of its on-line extent overran
        // each end by 6px.
        let (pad, pad_along) = if k.rem_euclid(2) == 0 {
            (0.5 * s, ap)
        } else {
            (ap, 0.5 * s)
        };
        // Tile extent in this frame: `n` fixes where the two walls sit, `u` only seeds the
        // fallback span and the mid-point that decides which room owns which end.
        // Only the `u` extent is needed: the candidate walk below derives each wall's `n`
        // offset from the tiles itself.
        let (mut lo_u, mut hi_u) = (f64::MAX, f64::MIN);
        for t in &self.tiles {
            let p = t.center(s);
            let pu = p.0 * u.0 + p.1 * u.1;
            lo_u = lo_u.min(pu);
            hi_u = hi_u.max(pu);
        }
        let mid_u = 0.5 * (lo_u + hi_u);
        // The probe segment must reach PAST both rooms' floors, or a border crossing lands
        // outside it and is never found — with a fixed 2-tile reach, 6D's crossing sat 18px
        // beyond the probe's end.
        let (mut far_lo, mut far_hi) = (lo_u, hi_u);
        for side in [self.a, self.b] {
            for h in areas.room_cells(side) {
                let p = h.center(s);
                let pu = p.0 * u.0 + p.1 * u.1;
                far_lo = far_lo.min(pu);
                far_hi = far_hi.max(pu);
            }
        }
        let (far_lo, far_hi) = (far_lo - 2.0 * s, far_hi + 2.0 * s);
        let span = far_hi - far_lo;
        // Each room's mean position along the wall — which end it owns. Loop-invariant, so it is
        // computed once here rather than per candidate: the two inline copies this replaces ran
        // a full `room_cells` scan per candidate per side per wall pass, and had already drifted
        // (one treated an empty side as `continue`, the other silently took the high end).
        let mean_u: [Option<f64>; 2] = [self.a, self.b].map(|side| {
            let (mut acc, mut cells) = (0.0f64, 0usize);
            for h in areas.room_cells(side) {
                let p = h.center(s);
                acc += p.0 * u.0 + p.1 * u.1;
                cells += 1;
            }
            (cells > 0).then(|| acc / cells as f64)
        });
        // Candidate wall lines: the lattice lines that bound each tile in this frame, i.e.
        // every tile centre offset by `pad` either way. Exact rather than a guessed step —
        // stepping by an apothem lands halfway, on a tile CENTRE line, which is not a wall.
        let mut cands: Vec<f64> = Vec::new();
        for t in &self.tiles {
            let p = t.center(s);
            let pn = p.0 * n.0 + p.1 * n.1;
            for o in [pn - pad, pn + pad] {
                if !cands.iter().any(|c| (c - o).abs() < 1e-6) {
                    cands.push(o);
                }
            }
        }
        cands.sort_by(f64::total_cmp);
        // Halve the walk step — but ONLY in the edge families. The two families differ, and
        // conflating them admits walls on lines that carry no hex edge:
        //
        // - odd `k` (`n` an edge normal): centre projections are multiples of the apothem, so
        //   `centre ± ap` lands on the true line set and the midpoints between consecutive
        //   candidates are themselves genuine edge lines — in a honeycomb an edge's extension
        //   passes through the offset neighbour's centre. Without these the walk stepped a full
        //   `2*ap` and jumped over the line that bounds the passage (5D<->6D).
        // - even `k` (`n` a vertex direction): centre projections are multiples of `1.5s` and
        //   candidates are `1.5s*r ± s/2`, so consecutive gaps already alternate `s` and `s/2`
        //   — every real line is present. A midpoint here is `1.5s*r`, a tile CENTRE line, or
        //   `1.5s*r + 0.75s`, which is no line at all. The acceptance test cannot reject either:
        //   a centre line lies over claimed ground and reaches both borders just as readily.
        if k.rem_euclid(2) == 1 {
            let mut mids: Vec<f64> = cands
                .windows(2)
                .map(|w| 0.5 * (w[0] + w[1]))
                .filter(|m| !cands.iter().any(|c| (c - m).abs() < 1e-6))
                .collect();
            cands.append(&mut mids);
            cands.sort_by(f64::total_cmp);
        }
        let mut out = [Vec::new(), Vec::new()];
        // The two walls are chosen in sequence, not independently: the second must stay at
        // least a tile's width from the first. Searching each side alone let both walk inward
        // onto the same or adjacent lines — measured over 120 maps, 9 corridors came out with a
        // sub-tile lane and one with the two walls exactly coincident.
        let mut picked: Option<f64> = None;
        for (i, outermost_first) in [false, true].into_iter().enumerate() {
            // Walk candidates from the OUTERMOST inward and take the first line that is a
            // valid wall. Validity is all-or-nothing, never clipped:
            //
            //  1. it reaches BOTH room borders — a wall that stops short of a border is the
            //     wrong path, not a shorter wall;
            //  2. its whole border-to-border run lies within the corridor's claimed tiles (R4).
            //
            // Clipping a partial line to "what fits" instead produced walls that ended in open
            // ground and let a wall sit on an outer line only a fragment of which had tile
            // under it.
            let order: Vec<f64> = if outermost_first {
                cands.iter().rev().copied().collect()
            } else {
                cands.clone()
            };
            for off in order {
                // Keep the pair apart: a lane narrower than one tile is not a passage.
                if picked.is_some_and(|p| (off - p).abs() < s - 0.01) {
                    continue;
                }
                let base = (n.0 * off, n.1 * off);
                let at = |t: f64| (base.0 + u.0 * t, base.1 + u.1 * t);
                let (fa, fb) = (at(far_lo), at(far_hi));
                // Border-to-border span. Both ends must come from a real border crossing;
                // a side with no room border (organic or hall) falls back to the tile bound.
                let (mut span_lo, mut span_hi) = (None, None);
                for (si, side) in [self.a, self.b].into_iter().enumerate() {
                    // Which end this room owns, from its own floor — the same tile fact
                    // everything else uses. Testing both ends per room collapses the span.
                    let Some(mu) = mean_u[si] else { continue };
                    let low_end = mu < mid_u;
                    let Some(sh) = areas.room_border(side) else {
                        // An organic or hall-shaped side has NO border to reach, so it cannot
                        // be required to supply a cap: that end falls back to the tile bound
                        // and still counts as satisfied. Demanding a crossing here erased the
                        // 100<->4D wall, whose far side is organic.
                        if low_end {
                            span_lo = Some(lo_u - pad_along);
                        } else {
                            span_hi = Some(hi_u + pad_along);
                        }
                        continue;
                    };
                    let ts: Vec<f64> = sh
                        .segment_crossings(fa, fb)
                        .into_iter()
                        .map(|t| far_lo + t * span)
                        .collect();
                    if low_end {
                        span_lo = ts.iter().copied().filter(|&t| t <= mid_u).reduce(f64::max);
                    } else {
                        span_hi = ts.iter().copied().filter(|&t| t >= mid_u).reduce(f64::min);
                    }
                }
                // Both ends must be real: a border crossing, or a borderless side's tile bound.
                // (Previously an `unwrap_or` supplied a fallback here that the acceptance test
                // 25 lines below then rejected anyway — dead by construction.)
                let (Some(lo), Some(hi)) = (span_lo, span_hi) else {
                    continue;
                };
                // Two crossings landing within half a hex of each other is a graze, not a wall.
                if hi - lo < 0.5 * s {
                    continue;
                }
                // Two conditions, both required, and the outermost candidate that meets them
                // wins:
                //
                //  1. the line reaches both room borders (a line stopping short is the wrong
                //     path, not a shorter wall);
                //  2. it never crosses UNCLAIMED ground. It may run alongside either room's own
                //     floor — the annotated reference shows exactly that — but a stretch over
                //     rock is what "crosses unclaimed tiles" means, and walking one lattice row
                //     inward is the fix.
                //
                // Testing containment against the corridor's OWN tiles instead was too strict
                // and rejected every candidate here; ignoring the question entirely let the
                // wall run over rock. Claimed-or-not is the discriminator.
                // Step off the wall along its OWN inward normal — `+n` for the low wall, `-n`
                // for the high one — by half a cell, the same nudge `decor::wall_over` uses to
                // ask which cell a wall sample backs onto. Aiming at the tile centroid instead
                // ran nearly PARALLEL to the wall near its ends, so the sample barely left the
                // edge it was meant to disambiguate; and the offset was 0.6px absolute where
                // the established primitive uses `0.6 * s`.
                let inward = if i == 0 { (n.0, n.1) } else { (-n.0, -n.1) };
                let over_claimed = (0..=12).all(|q| {
                    let t = lo + (hi - lo) * (q as f64 / 12.0);
                    let p = at(t);
                    let g = (p.0 + inward.0 * 0.6 * s, p.1 + inward.1 * 0.6 * s);
                    areas.owner_of(Hex::at(g, s)).is_some()
                });
                if over_claimed {
                    out[i] = vec![at(lo), at(hi)];
                    picked = Some(off);
                    break;
                }
            }
        }
        out
    }
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
        let (e0, e1) = Hex::edge_corners(k);
        ((cs[e0].0 + cs[e1].0) / 2.0, (cs[e0].1 + cs[e1].1) / 2.0)
    };
    Some(match areas.room_border(side) {
        Some(sh) => sh.nearest_on_wall(raw),
        None => raw,
    })
}

/// The landing of `tiles` on `side`'s fitted border: the per-tile arrow marks (R3) through the
/// same `nearest_on_wall` locus the spine anchors use — the marks and the spine ends must never
/// pick different border points for the same contact — plus the contact as merged [`Gap`]s.
///
/// **Every touched edge feeds the gap, collapsed tile or not.** R3's collapse says where a tile's
/// arrows converge, not how wide its contact is; a gap derived from collapse points alone would
/// be a gap of zero width on exactly the tiles that present the most wall.
fn attachment(areas: &Areas, tiles: &[Hex], touches: &[[bool; 6]], side: usize, s: f64) -> Landing {
    let Some((sh, per)) = areas.room_border_per(side) else {
        return Landing::default();
    };
    let mut marks: Vec<(usize, Mark)> = Vec::new();
    let mut verts: Vec<(Point, Point)> = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    for (i, (t, touch)) in tiles.iter().zip(touches).enumerate() {
        let cs = t.corners(s);
        let collapse = TileAxis::collapse(touch);
        if let Some(c) = collapse {
            marks.push((i, Mark::Point(sh.nearest_on_wall(cs[c]))));
        }
        for k in (0..6).filter(|&k| touch[k]) {
            let (e0, e1) = Hex::edge_corners(k);
            // The same `nearest_on_wall` locus the marks and the spine anchors use — the span
            // and the arrows must never pick different border points for the same contact.
            let (a, b) = (sh.nearest_on_wall(cs[e0]), sh.nearest_on_wall(cs[e1]));
            if collapse.is_none() {
                marks.push((i, Mark::Bar(a, b)));
            }
            verts.push((cs[e0], cs[e1]));
            // The shorter way round: a hex edge is a small fraction of any room's perimeter,
            // so the short arc is never the ambiguous choice — and taking it is what keeps the
            // span ON the contact rather than running the long way round the room.
            spans.push(Span::shorter(sh.wall_param(a), sh.wall_param(b), per));
        }
    }
    // Two edges of the contact share a vertex EXACTLY — same input point, same projection — so
    // their spans abut and merge into one. The tolerance therefore only has to absorb float
    // noise; it is never bridging a real stretch of surviving wall. `merge_groups` rather than
    // `merge`, so which edges joined which stretch is the merge's own answer — a landing that
    // does come out as two stretches sizes each door from its own contact.
    let gaps = merge_groups(&spans, per, 1e-6)
        .into_iter()
        .filter_map(|(span, members)| {
            // This stretch's chord: the two farthest-apart of its own edges' tile vertices.
            // Farthest-apart rather than first-and-last, because the vertices arrive in tile
            // order and a bending run can revisit a parameter.
            let pts: Vec<Point> = members
                .iter()
                .flat_map(|&k| [verts[k].0, verts[k].1])
                .collect();
            farthest_pair(&pts).map(|chord| Gap {
                span,
                chord,
                edges: members.len(),
            })
        })
        .collect();
    Landing { marks, gaps }
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
                centerline,
                tiles,
                axes,
            }
        })
        .collect()
}
