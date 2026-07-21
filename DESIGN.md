# Design: how `maps` generates deterministic caves, glades and ruins

This document describes the generation pipeline in `crates/maps-core`. The
cave/glade process follows watabou's Cave Generator as reverse-engineered in
Boris the Brave's writeup —
[How does Cave/Glade Generator work?](https://www.boristhebrave.com/2023/11/19/how-does-cave-glade-generator-work/)
— reimplemented from that public description. The ruins process is our own
extension. Section headings note the implementing module.

## Determinism model (`lib.rs`)

Everything derives from a master `u64` seed, split via splitmix64 into three
independent, individually overridable streams:

| Stream | Seeds | Drives |
|---|---|---|
| **shape** | `shape_seed` | tags (when rolled), area growth, topology, outline smoothing, water, stones |
| **decor** | `decor_seed` | hatch fans / stipple dots / tree canopies / masonry, including their stacking order |
| **name** | `name_seed` | the title (unless overridden verbatim by `title`) |

Rules that keep output byte-identical across platforms (native and wasm):

- Every random choice flows through a seeded PCG-64 in a **fixed call
  order**; collections are never iterated in hash order when the result
  feeds the RNG (cells are sorted first).
- Random tags come from their own sub-stream of the shape seed, so
  supplying the same tags explicitly reproduces the identical map — the
  basis of replication and of the web UI's per-family defaults.
- Draws happen unconditionally where a feature might be disabled (e.g. the
  water noise salt), so toggling one option never shifts another's stream.
- Seeds are `u64` and cross the JavaScript boundary as strings (numbers
  corrupt integers above 2^53).
- `Tags::random` gives **every family a concrete tag**, making the
  generator the single source of truth for what an untouched family means.
- All geometry stored on the map is **quantized to exact tenths of a
  pixel** (radii and opacities to hundredths) the moment it is produced,
  and the SVG writer prints coordinates with pure integer fixed-point
  formatting — the stored values, the rendered bytes, and the float
  pipeline's rounding are all one convention by construction (and integer
  printing is far cheaper than exact float-to-decimal conversion).

## The cave/glade pipeline

### 1. Tags as parameter presets (`tags.rs`, `growth.rs::resolve`)

As in the original, tags are switches/presets resolved into numeric
parameters: area count by size (small 2–3, medium 3–8, large 9–19), area
sizes by layout (`hub`: one 60–79-cell chamber plus satellites; `chamber`:
even 11–14±2; `burrow`: `10 + 80·((r+r+r)/3)³`), and the growth exponent by
shape (below). `water_level`/`ruins_level` fine-tune the active tag's
default; `dry`/`organic` are absolute.

### 2. Hex grid (`grid.rs`)

Pointy-top axial-coordinate hexes on a board sized from the total target
area. (The article describes a DCEL; we get the same adjacency and boundary
walks directly from axial arithmetic.)

### 3. Seed growth (`growth.rs`)

Each area starts at a seed cell and accretes one neighbouring cell at a
time. Candidates are weighted `c^gamma` where `c` is how many neighbours
are already in the area: `cavities` pins gamma at 6 (round blobs), `coral`
uses negative gamma (tendrils), `chaotic` prefers cells with exactly two
connections. Growth preserves a **one-cell buffer** between areas — those
gap cells are the future doorways. An area stops at its tag-resolved target
size (`advance_single`).

Seeds are placed in **three drops** across **four overlapping corner
sections** (`grow_areas`, our refinement over seeding everything centrally).
Each section is the board's central third pushed one sixth toward a corner —
about a quarter of the board, off the rim, all four sharing the centre. A
drop draws one section and seeds its areas there; areas are split into even
thirds by count with **dungeon rooms (and their symmetry orbits) forced into
the first drop** for clean space (orbits ignore the section and radiate from
a central point where symmetric wings fit). Spreading seeds this way stops
them crowding and starving each other. Because spread seeds sit further
apart, targets are boosted 50% (`GROWTH_BOOST`) so a grown area still reaches
its neighbours' doorway range and connects, rather than finishing short and
being dropped as unreachable (§ "Exits" and `keep_largest_component`).

### 4. Doorways (`topology.rs`)

Gap cells adjacent to two areas are grouped per area-pair, then culled:
`tree` keeps a random spanning tree (no loops), `connected` breaks one edge
of every triangle of mutually adjacent areas. One door cell is chosen per
surviving pair.

### 5. Corridors (`topology.rs`)

Areas are randomly chosen to shrink into corridors (probability grows with
door count; `burrow` raises it; a hub's main chamber is exempt). Shrinking
removes random cells while flood-fill confirms the remainder stays
connected and every door/exit keeps contact — converging to winding
width-1 passages.

### 6. Exits (`topology.rs`)

The article picks exits from eligible **border cells**, weighted toward the
rim — so in it a room can reach the board edge. Ours cannot: areas seed in
corner sections that stop short of the rim (see §3) and never touch it, so no
room cell sits on the border and an exit cannot be a tagged border cell. It
is instead a **carved passage** from a room outward to the rim. Candidate attach cells
(any area cell with a free outward neighbour) keep the article's
squared-distance-from-centre weighting; a passage then walks outward cell by
cell until it reaches the board rim. A walk that gets boxed in before the rim
would dead-end mid-map, so that **candidate is discarded** — dropped from the
exit-candidate list, not removed from the map; the area stays and can still
host an exit from another of its cells. Counts: `sealed` 0, `entrance` 1,
`passage` 2, `junction` 3–4.

### 7. Water as terrain elevation (`water.rs`)

A hand-rolled two-octave value noise (deterministic integer-lattice hash)
assigns each floor cell an elevation. The water level is a **fill
fraction**: the waterline is that quantile of the map's elevations, so 0 is
dry, 0.5 floods the lowest half, 1 submerges everything. Two derived bands
follow the same waterline: deep water below `level − 0.15` and a mud fringe
below `level + 0.06` (only where it touches a real pool). Pool outlines run
through the same smoothing pipeline as walls but with far less jitter —
water lies glassy against rough rock.

### 8. Boundary tracing and smoothing (`outline.rs`)

The floor (areas ∪ doors ∪ exit passages) is one cell set; a cell-following
contour walk produces closed loops — outer walls and interior pillars —
with pinch corners handled unambiguously. Each loop then goes through the
article's smoothing sequence:

1. subdivide every edge;
2. Laplacian smoothing (*bumpiness*);
3. pull tunnel-cell vertices toward their cell centres (*narrowing* — what
   makes corridors, doors and exit passages read as tight passages);
4. per-vertex jitter (*irregularity*);
5. two rounds of subdivide + finer jitter (*roughness*);
6. Chaikin corner cutting.

We add two closing steps: decimation (drop sub-pixel points) and
**bowtie removal** — a spatially-hashed self-intersection scan that cuts
the smaller lobe at any crossing, guaranteeing simple loops whatever
upstream jitter or projection did.

#### The simple-loop invariant: prevent where a signal exists, enforce always

Self-intersecting loops ("bowties") have two distinct causes, handled at
different depths:

1. **Room-boundary crossings** (two ruin shapes' wall loci intersecting,
   or projected walls folding at door mouths) are *prevented at the cell
   level*, using the ownership map: seam cells (adjacent to another area)
   and contested cells (inside a second ruin's geometry) are excluded from
   projection, the blend ramp fades projection near every ownership
   transition, and hall projection fades with displacement. Cells carry
   the signal, so generation can simply not produce the fold.
2. **Jitter micro-folds** — 2–5-point sub-pixel lobes created by random
   vertex noise mid-wall, inside a single area — carry no cell-level
   signal at all. Measured with isotropic jitter they occurred on *plain
   organic* maps more often than ruins maps (~2.2 vs ~1.0 cuts per large
   map). The fix is generative but geometric, not cell-based: **jitter
   displaces each vertex along its local wall normal only**. Perpendicular
   displacement cannot reorder vertices along the curve, so the jitter
   passes cannot fold the loop by construction. Measured result: organic
   maps dropped to zero cuts; ruins maps to ~0.13 per map (residual
   projection edge cases).

`remove_bowties` stays as the unconditional final guarantee regardless:
prevention reduces its workload to nearly nothing, but no generative
scheme can *prove* global simplicity (thin necks, future geometry), and
the scan is effectively free (spatial hash, early exit when clean).

### 9. Decoration (`decor.rs`)

All decoration draws from the decor stream and is **shuffled** at the end,
so stacking/overlap order is seed-decided rather than a cascade in the
boundary-walk direction.

- **Cave walls**: cone fans — five parallel strokes of growing length per
  fan, scattered along the wall like trees along a tree line. Each fan has
  an opaque footprint that hides fans beneath it; the floor mask clips
  whatever falls inside the cave, so fans that start deep show only their
  longest strokes. A translucent shadow band hugs the outside of every
  wall, above the fans.
- **Glade walls**: tree canopies — star polygons with lobe count scaling
  with radius and a recessed inner ring, laid in three depth bands that
  lighten toward the clearing (the background is the darkest layer of all;
  the cave palette sets those depth colours to one beige, which is why no
  shading shows there).
- **Stones**: small irregular polygons in open cells.
- **Ruin floor patterns** (`pattern` tag: mosaic/truchet/islamic/plain):
  tile patterns drawn on ruin-area cells so architecture reads against
  organic floor, ported from `plan/hex-tile-pattern.md` onto our
  pointy-top grid. *Mosaic* fills shrunk cell hexes (grout gaps) with
  radial wave shades from each ruin's centroid plus occasional rng
  "replaced" tiles; *Truchet* joins edge midpoints with quadratic bezier
  ribbons (knot or maze wiring chosen per ruin, rotation per cell), flowing
  continuously across shared edges; *Islamic* uses Hankin's
  polygons-in-contact — rays projected inward from edge midpoints at a
  per-map angle (30° stars or 45° rosettes) trimmed at their
  intersections. Drawn just above the floor fill so water floods over
  them; all randomness rides the decor stream.

### 10. Naming (`naming.rs`)

A small Tracery-style grammar with cave and woodland lexicons; water
presence biases toward damp names. An explicit `title` bypasses generation
without consuming the name stream.

### 11. Rendering (`render.rs`)

Layer order: background → trees → floor fill → mud → water → deep water →
grid overlay (hex, square or none) → stones → hatching/dots → shadow band →
masonry → wall stroke → title. Nonzero winding everywhere so overlapping
loops union; an inverse-floor mask (`#rock`) restricts wall decoration to
rock. The floor path — the largest string in the file — is defined once in
`<defs>` and instantiated five times with `<use>` (clip, mask, fill,
shadow, border), which also gives the shadow band its offset: translated
down-right and clipped to the floor, so the border's drop shadow falls
only on room contents — the rock-side decoration stays clean — and the
border reads as floating above the rooms, as in the original. The seed/tags caption is deliberately not part of the SVG — that
information lives in the CLI output, the web readout and permalinks.

## The ruins extension (`ruins.rs`)

Ruins replace a fraction of the grown areas with geometric architecture —
`ruins_level` (0..1) picks how many, the way the water level picks how much
floods. The guiding principle, arrived at after fighting rendering-level
patches: **ruins are cells, like every other node**. Geometry is realised
on the hex grid *before* any outline exists, so unions, passage widths and
wall thickness follow from the same grid rules as organic areas.

### Fitting

Selected rooms get an area-preserving rectangle or circle (50/50), centred
on the cell centroid and deliberately fitted at ~0.8× area so neighbours
merge by intent rather than constantly. Selected corridors get a straight
or arcing hall between their two farthest cells; fits that would misbehave
are rejected and the area stays organic — a corridor too contorted for its
centreline, an arc radius under 2.5 cells (its raster would wrap into a
ring), or any raster that fails validation below.

### Rasterizing (growing like a node)

The shape claims the hexes it covers (free cells or its own; a small margin
keeps the traced boundary outside the exact geometry so projection only
ever pulls walls inward). Door-adjacent original cells are kept so no door
is orphaned. The result must pass three checks or the area reverts to
organic: connected, every door still adjacent, and **no enclosed rock**
(a flood-fill test — a ring-shaped raster would pinch the floor around its
pocket shut).

Because claimed cells may reach another area's neighbourhood, **touching
ruins union at the cell level**: the contour walk traces one loop around
both and no interior border ever exists. The threshold for "one room" is
the grid's own one-cell buffer.

### Projection: making cells look like geometry

During outline smoothing, boundary vertices of ruin cells are projected
onto the fitted shape and locked against jitter — rectangles come out
straight, circles arc, halls run clean. Four safeguards keep projection
from ever fighting the grid:

- **Blend ramp**: projection fades in over the last few vertices of each
  ruin run, so transitions to organic wall (door mouths, merge seams)
  cannot fold the loop.
- **Seam cells** (adjacent to another area) stay organic: merged throats
  keep full cell width.
- **Contested cells** (inside a *second* ruin's geometry) stay organic:
  two shapes' wall loci never extend across each other, which would tie
  the border into a bowtie.
- **Hall displacement falloff**: a hall vertex projects crisply only when
  the move is under half a cell, fading to organic beyond 1.5 cells —
  radial side walls can't leap across the passage.

Bowtie removal (pipeline step 8) backs all of this with a hard guarantee
of simple loops.

### Ruin decoration

Wall samples are classified by the cell just inside the wall (pixel→hex
lookup): ruin-owned wall sections swap their decoration — **faded stipple
dots** in caves (denser, larger and darker against the line, fading out)
and **masonry tiles** in glades (an overlapping course recessed under the
border stroke, seed-shuffled stacking, mitered L-shaped quoins at sharp
corners). Organic sections keep fans/canopies, switching exactly at the
ownership boundary — so a half-merged structure reads geometric on one
side and wild on the other.

### Dungeon areas (`lib.rs`)

The organic → ruin spectrum extends to a third state, **dungeon**, nested
inside ruins. `ruins_level` still sets how much of the map is geometric;
`dungeon_level` (0..1) is the fraction of *those* geometric areas promoted
from weathered ruin to clean dungeon room:

    organic  = 1 − ruins_level
    ruin     = ruins_level · (1 − dungeon_level)
    dungeon  = ruins_level · dungeon_level

The `dungeon` tag sets the level's default (0.6); `natural` forces 0.

Unlike ruins (reshaped after the fact), a dungeon room is **built door-ready
from the start**:

- **Classified first.** Area kinds (organic/ruin/dungeon) are assigned to the
  growth slots *before* growth (`lib.rs::classify_slots`), and every area
  carries its kind (`growth::Areas::kind`).
- **Staggered simultaneous growth.** `growth::grow_areas` seeds a few areas
  every 1–3 rounds and advances all of them together each round, so which
  seeds land first — and thus which areas win the open space and grow large —
  varies per map (natural size variety). Every area keeps a one-cell rock gap
  from every other (`placeable`); the gaps become doorways. Because dropping an
  under-sized area could orphan a neighbour, growth ends by keeping only the
  largest connected component (`keep_largest_component`), so the door graph
  always spans the map.
- **Grown as its true geometry.** A dungeon area grows as a circle (one
  concentric ring per round) or a rectangle (one side-strip per round, random
  side order). Every increment is all-or-nothing: if any cell of the next
  ring/strip is blocked, that increment is refused — a rectangle tries its
  other sides, a circle stops. The wall (`RuinShape` on `Areas::shape`,
  `ROOM_WALL_PAD` outside the outermost cell centres, derived from the final
  cells) is the true geometry cutting through the boundary hexes; the cells are
  just the ownership raster.
- **Symmetric wings as sibling orbits.** `symmetry::choose` picks a per-map
  plan; some dungeon areas become orbit **generators** whose sibling copies
  grow in **lockstep** — each round the generator adds its increment and every
  sibling adds the transformed increment (`symmetry::Xform`), committed only if
  all members fit, so a wing is always exactly symmetric and self-sizing (when
  one sibling is blocked, all stop). All symmetry is one mechanism: bilateral
  (reflect), 180°, translated ("identical") and radial (3-/6-fold) are just
  different transform sets about a shared centre. Hex lattice rotations are
  exact only at order 2/3/6, and a render-space rectangle only survives
  reflection/180°, so radial orbits grow disks. Generators are seeded first
  (they need clean space); the copies are ordinary dungeon areas that get
  doors, walls and decor like any room.
- **Walls are traced from the shape, not the raster.** A hex raster cannot
  tile an axis-aligned rectangle cleanly — the staggered rows leave corner
  cells unfilled and edges ragged — so *projecting* each traced vertex left
  the wall hostage to the raster (chamfered corners, eroded edges wherever a
  neighbour's geometry overlapped a cell). Instead, **shape-tile tracing**
  (`outline::splice_dungeon_runs`): a dungeon room's boundary always forms
  contiguous runs in the traced loop, and each run is **replaced wholesale**
  by a resampling of the room's exact wall between the run's endpoints —
  rect corners emitted exactly with straight edges, circle arcs sampled every
  half-cell. The raster vertices are discarded, so the wall is exact by
  construction. Spliced vertices carry the room's shape and stay locked
  through every later pass (smoothing, jitter, Chaikin are all identities on
  them). A dungeon room is its own convex loop (rooms never fuse), so this
  can't fold; `remove_bowties` still backstops the rare shape-overlap seam.
- **Doorway mouths, openings and lips** (`doorway.rs`). Doors onto dungeon
  rooms are clustered into **mouths** — two doors merge only when their cells
  are hex-adjacent *and* share a dungeon room (adjacent doors onto *different*
  neighbours pierce different walls and stay separate). Each mouth has a
  controlled **opening** width — one hex per member door (capped at three,
  never below one) — and this single width drives everything: the door cells
  (and each dungeon exit's stub) get a `StraightHall` **plug** of that
  half-width in the ruin cell map (the crisp **lip**, fading organic down the
  corridor via the hall displacement falloff), and the splice **snaps** its
  run endpoints to the mouth's jambs (opening centre ± half-opening on the
  wall), so the gap cut into the exact wall is exactly the doorway the bar
  spans — sprawling gaps close, pinched passages open. Wall decor is
  classified **geometrically** (`RuinShape::wall_dist`): samples on a dungeon
  room's perimeter or a lip stay clean — a cell lookup would hatch the
  unfilled corners organic. Dungeon rooms are also exempt from corridor
  shrinking and still take floor patterns (`pattern` tag).

Every mouth onto a dungeon room is drawn from its anchor (`render::door_layer`
consumes `map.mouths`): a **door bar spanning the opening** with a dark **jamb
cap** at each end, emitted *under* the wall border so the caps merge into the
wall line, the bar set `0.25·s` outside the wall — inside the lip. Each door
carries a `DoorStyle`: `Wood` (plain leaf), `Metal` (leaf + band),
`Portcullis` (a row of rings), or `Open` — no glyph at all, a bare framed gap
(15% of doors directly between two dungeon rooms roll open; not every room
needs a door). The bar spans the mouth's full opening width with one leaf per
hex, so a two- or three-hex mouth becomes a **double/triple door** — leaf
rectangles seamed per hex, or a longer portcullis — taking the strongest
member style (portcullis > metal > wood > open).

Placement is **flush** (`doorway::mouth`): a mouth touching one dungeon room
sits in that room's wall — perpendicular to a rectangle's pierced wall (chosen
by passage direction; position ratios misclassify corner-adjacent doors) or on
the circle tangent. A door between **two** dungeon rooms is never averaged
askew across the gap: if the two pierced walls face each other squarely
(parallel tangents, openings collinear) the lip sits centered between them
with the throat width unchanged; otherwise the door goes flush into the wall
the passage hits most squarely, and the other room keeps a plain gap.

Determinism is per-seed only (same seed + options → identical map, including
wasm); all dungeon decisions ride the shape stream. (Connector-passage door
placement and locally-mirrored symmetric wings are the remaining steps; see
`plan/dungeon-mode.md`.)

## Verification

`crates/maps-core/tests/basic.rs` pins the invariants (determinism, door
connectivity, sub-seed stream isolation, water monotonicity, ruin/decoration
semantics); `tests/trace.rs` holds regression tests on user-reported seeds
for minimum passage width and loop simplicity. The wasm build is checked
byte-identical against the native `examples/cave.svg`.
