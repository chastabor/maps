//! A window of the hex grid, cell by cell: which area owns each cell and which connection's run
//! occupies it. Prints an ASCII picture and writes the same window as an SVG.
//!
//! The ASCII form is the one to paste into a plan or a doc — it is the only view in the repo where
//! a pocket's *lattice* facts are legible: exactly which cells a passage took, and whether a cell
//! of rock survives between two of them. The rendered views draw geometry, and geometry is what
//! hides a missing barrier (`plans/tile-corridor-render.md`, the seed-82 pocket). The SVG is the
//! same window with the hexes in their true positions, for when the shape of the pocket matters as
//! well as its cells.
//!
//! Columns are `x = 2q + r` and rows are `r`, so a cell's neighbours are `(x±2, r)` and
//! `(x±1, r±1)` — the offset text layout that makes hex adjacency readable:
//!
//! ```text
//!       -13 -12 -11 -10  -9  -8  -7
//! r-10        <2>     10o     <6>
//! r-9    0D      <2>     <6>      2D
//! r-8        0D     <0>      2D
//! ```
//!
//! ```text
//! SEED=82 TAGS=large,ruins,dungeon,separate R0=-12 R1=-4 X0=-14 X1=6 OUT=window.svg \
//!     cargo run -p maps-core --release --example cell_map
//! ```

mod common;

use common::env;
use maps_core::grid::{HEX_SIZE as S, Hex};
use maps_core::render::{PALETTE, hex_points};
use maps_core::{AreaKind, CaveMap};
use std::collections::HashMap;
use std::fmt::Write as _;

/// What occupies a cell: an area's floor, one connection's run or apron, or an exit stub.
enum Role {
    Area(usize, AreaKind),
    Run(usize),
    Apron(usize),
    Exit,
}

impl Role {
    /// The ASCII tag: `5D` an area, `<3>` a connection's run, `(3)` its apron, `###` an exit.
    fn tag(&self) -> String {
        match *self {
            Role::Area(i, kind) => format!("{i}{}", kind.letter()),
            Role::Run(i) => format!("<{i}>"),
            Role::Apron(i) => format!("({i})"),
            Role::Exit => "###".into(),
        }
    }

    fn fill(&self) -> &'static str {
        match *self {
            Role::Area(i, _) => PALETTE[i % PALETTE.len()],
            // Run and apron in the growth view's passage white, the apron paler: it is room floor
            // the passage crosses, not floor the passage took.
            Role::Run(_) => "#f2f2ea",
            Role::Apron(_) => "#cfcfc4",
            Role::Exit => "#ff8c42",
        }
    }
}

/// The column-layout convention shared by both views: only cells whose parity matches exist
/// (`x` and `r` move together), and `x = 2q + r` inverts to `q`.
fn hex_at(x: i32, r: i32) -> Option<Hex> {
    ((x - r).rem_euclid(2) == 0).then(|| Hex { q: (x - r) / 2, r })
}

/// Every occupied cell in the map, connections and exits taking precedence over ownership — a
/// claimed run cell is owned too, and the run is what one wants to see.
fn roles(m: &CaveMap) -> HashMap<Hex, Role> {
    let mut out: HashMap<Hex, Role> = HashMap::new();
    for i in 0..m.areas.count() {
        for h in m.areas.room_cells(i) {
            out.insert(h, Role::Area(i, m.areas.kind(i)));
        }
    }
    for (i, c) in m.topology.connections.iter().enumerate() {
        for (k, &h) in c.along.iter().enumerate() {
            out.insert(
                h,
                if k < c.apron_from {
                    Role::Run(i)
                } else {
                    Role::Apron(i)
                },
            );
        }
    }
    for e in &m.topology.exits {
        for &h in &e.stub {
            out.insert(h, Role::Exit);
        }
    }
    out
}

fn ascii(roles: &HashMap<Hex, Role>, x0: i32, x1: i32, r0: i32, r1: i32) {
    print!("      ");
    for x in x0..=x1 {
        print!("{:^4}", x);
    }
    println!();
    for r in r0..=r1 {
        print!("r{r:<4} ");
        for x in x0..=x1 {
            let cell = hex_at(x, r).map_or_else(String::new, |h| {
                roles.get(&h).map_or_else(|| ".".into(), |v| v.tag())
            });
            print!("{:^4}", cell);
        }
        println!();
    }
}

fn svg(roles: &HashMap<Hex, Role>, x0: i32, x1: i32, r0: i32, r1: i32) -> String {
    let cells: Vec<Hex> = (r0..=r1)
        .flat_map(|r| (x0..=x1).map(move |x| (x, r)))
        .filter_map(|(x, r)| hex_at(x, r))
        .collect();
    let corners: Vec<(f64, f64)> = cells.iter().flat_map(|h| h.corners(S)).collect();
    let (lo_x, hi_x) = corners
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p.0), h.max(p.0)));
    let (lo_y, hi_y) = corners
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p.1), h.max(p.1)));
    let (lo_x, lo_y) = (lo_x - 8.0, lo_y - 8.0);
    let (w, hgt) = (hi_x + 8.0 - lo_x, hi_y + 8.0 - lo_y);
    let mut s = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{lo_x:.1} {lo_y:.1} {w:.1} {hgt:.1}" width="{:.0}" height="{:.0}">"##,
        w * 3.0,
        hgt * 3.0,
    );
    let _ = write!(
        s,
        r##"<rect x="{lo_x:.1}" y="{lo_y:.1}" width="{w:.1}" height="{hgt:.1}" fill="#14161c"/>"##
    );
    for h in &cells {
        let role = roles.get(h);
        let fill = role.map_or("#20232b", |v| v.fill());
        let _ = write!(
            s,
            r##"<polygon points="{}" fill="{fill}" stroke="#0b0c10" stroke-width="0.4"/>"##,
            hex_points(*h)
        );
        let (cx, cy) = h.center(S);
        // Rock cells carry their coordinates, occupied ones their role — the coordinates are what
        // a plan quotes when it names the cell a barrier should sit on.
        let (label, colour) = match role {
            // The run/apron tags carry angle brackets and parentheses; escape so `<0>` is text
            // rather than a stray element.
            Some(v) => (v.tag().replace('<', "&lt;").replace('>', "&gt;"), "#14161c"),
            None => (format!("{},{}", h.q, h.r), "#55596b"),
        };
        let _ = write!(
            s,
            r##"<text x="{cx:.1}" y="{:.1}" font-family="monospace" font-size="4.6" fill="{colour}" text-anchor="middle">{label}</text>"##,
            cy + 1.7
        );
    }
    s.push_str("</svg>");
    s
}

fn main() {
    let seed: u64 = env("SEED", 82);
    let tag_str = common::tags_env();
    let (r0, r1): (i32, i32) = (env("R0", -12), env("R1", -4));
    let (x0, x1): (i32, i32) = (env("X0", -14), env("X1", 6));
    let out = std::env::var("OUT").unwrap_or_else(|_| "cell_map.svg".to_string());
    let m = common::generate(seed, &tag_str);
    let roles = roles(&m);
    println!("seed {seed}  tags {tag_str}   <i> = connection i run, (i) = its apron, ### = exit");
    ascii(&roles, x0, x1, r0, r1);
    std::fs::write(&out, svg(&roles, x0, x1, r0, r1)).expect("write svg");
    println!("-> {out}");
}
