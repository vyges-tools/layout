//! Stress + oracle for contour tracing on real layout.
//!
//! Usage: `cargo run --release --example stress_contour -- <layout.gds> [top_cell]`
//!
//! For every layer it: self-unions the layer (`boolean_poly … Or` []), traces the
//! merged boundary, and checks the **area oracle** — the signed area of the traced
//! rings must exactly equal the area of the union tiles. Prints per-layer ring/tile
//! counts and wall-clock, so a dense metal layer becomes a real correctness + scale
//! test rather than a synthetic one.

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use vyges_layout::boolean::{boolean_poly, Op};
use vyges_layout::contour::trace_contours;
use vyges_layout::flatten;
use vyges_layout::gds::{Cell, Element, Library};
use vyges_layout::geom;

fn is_manhattan(pts: &[(i32, i32)]) -> bool {
    let n = if pts.len() >= 2 && pts.first() == pts.last() { pts.len() - 1 } else { pts.len() };
    (0..n).all(|i| {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % n];
        x1 == x2 || y1 == y2
    })
}

fn area2(ring: &[(i32, i32)]) -> i64 {
    let n = ring.len() - 1;
    let mut a = 0i64;
    for i in 0..n {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[i + 1];
        a += x1 as i64 * y2 as i64 - x2 as i64 * y1 as i64;
    }
    a
}

/// The one top cell (referenced by nobody), else the cell with the most elements.
fn top_cell<'a>(lib: &'a Library, arg: Option<&str>) -> &'a Cell {
    if let Some(name) = arg {
        if let Some(c) = lib.cells.iter().find(|c| c.name == name) {
            return c;
        }
    }
    let mut referenced: HashSet<&str> = HashSet::new();
    for c in &lib.cells {
        for el in &c.elements {
            match el {
                Element::Sref { sname, .. } | Element::Aref { sname, .. } => {
                    referenced.insert(sname.as_str());
                }
                _ => {}
            }
        }
    }
    lib.cells
        .iter()
        .filter(|c| !referenced.contains(c.name.as_str()))
        .max_by_key(|c| c.elements.len())
        .unwrap_or_else(|| lib.cells.iter().max_by_key(|c| c.elements.len()).unwrap())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: stress_contour <layout.gds> [top_cell]");
    let top_arg = args.next();

    let lib = Library::load_any(&path).expect("load layout");
    let top = top_cell(&lib, top_arg.as_deref()).name.clone();
    let flat = flatten::flatten(&lib, &top).unwrap_or_else(|_| top_cell(&lib, Some(&top)).clone());
    println!("layout: {path}   top: {top}   flat elements: {}", flat.elements.len());

    // bucket manhattan boundaries by (layer, datatype)
    let mut by_layer: BTreeMap<(i16, i16), Vec<Vec<(i32, i32)>>> = BTreeMap::new();
    let mut approx = 0usize;
    for el in &flat.elements {
        let (l, d, pts) = match el {
            Element::Boundary { layer, datatype, pts } => (*layer, *datatype, pts),
            Element::Box { layer, boxtype, pts } => (*layer, *boxtype, pts),
            _ => continue,
        };
        if is_manhattan(pts) {
            by_layer.entry((l, d)).or_default().push(pts.clone());
        } else if let Some(b) = geom::bbox(pts) {
            by_layer.entry((l, d)).or_default().push(b.as_boundary());
            approx += 1;
        }
    }
    if approx > 0 {
        println!("({approx} non-manhattan shapes bbox-approximated)");
    }

    println!(
        "\n{:>8} {:>8} {:>8} {:>8} {:>10} {:>10}  oracle",
        "layer/dt", "shapes", "tiles", "rings", "union_ms", "trace_ms"
    );
    let (mut tot_union, mut tot_trace) = (0.0f64, 0.0f64);
    let mut worst_layer = (String::new(), 0usize);
    let mut all_ok = true;
    for ((l, d), polys) in &by_layer {
        let t0 = Instant::now();
        let tiles = boolean_poly(polys, &[], Op::Or); // self-union
        let union_ms = t0.elapsed().as_secs_f64() * 1e3;
        let tile_area: i64 = tiles.iter().map(|r| r.area()).sum();

        let t1 = Instant::now();
        let rings = trace_contours(&tiles);
        let trace_ms = t1.elapsed().as_secs_f64() * 1e3;

        let ring_area2: i64 = rings.iter().map(|r| area2(r)).sum();
        let ok = ring_area2 == 2 * tile_area;
        all_ok &= ok;
        tot_union += union_ms;
        tot_trace += trace_ms;
        if tiles.len() > worst_layer.1 {
            worst_layer = (format!("{l}/{d}"), tiles.len());
        }
        println!(
            "{:>8} {:>8} {:>8} {:>8} {:>10.1} {:>10.1}  {}",
            format!("{l}/{d}"),
            polys.len(),
            tiles.len(),
            rings.len(),
            union_ms,
            trace_ms,
            if ok { "ok" } else { "AREA MISMATCH" }
        );
    }
    println!(
        "\ntotals: union {tot_union:.1} ms  trace {tot_trace:.1} ms  densest layer {} ({} tiles)",
        worst_layer.0, worst_layer.1
    );
    println!("area oracle: {}", if all_ok { "ALL LAYERS OK" } else { "FAILED" });
    std::process::exit(if all_ok { 0 } else { 1 });
}
