//! Engine: orchestrate the CLI verbs — `info`, `boolean`, `flatten`, `demo`.

use std::collections::BTreeMap;

use crate::boolean::{self, Op};
use crate::flatten;
use crate::gds::{Cell, Element, Library};
use crate::geom::{self, Rect};

/// Pick the cell to operate on: the named one, the single one, else an error.
fn pick<'a>(lib: &'a Library, top: Option<&str>) -> Result<&'a Cell, String> {
    match top {
        Some(t) => lib.cells.iter().find(|c| c.name == t).ok_or_else(|| format!("cell {t:?} not found")),
        None => match lib.cells.len() {
            1 => Ok(&lib.cells[0]),
            0 => Err("no cells in the library".into()),
            n => Err(format!("{n} cells; pass --top to choose one")),
        },
    }
}

/// Rectilinear polygons on a (layer, datatype). Only genuinely non-Manhattan shapes
/// (a diagonal edge) are bbox-approximated and counted — never silently dropped.
fn polys_on(cell: &Cell, layer: i16, datatype: i16) -> (Vec<Vec<(i32, i32)>>, usize) {
    let mut polys = Vec::new();
    let mut approx = 0;
    for el in &cell.elements {
        let (l, d, pts) = match el {
            Element::Boundary { layer, datatype, pts } => (*layer, *datatype, pts),
            Element::Box { layer, boxtype, pts } => (*layer, *boxtype, pts),
            _ => continue,
        };
        if l != layer || d != datatype {
            continue;
        }
        if is_manhattan(pts) {
            polys.push(pts.clone());
        } else if let Some(b) = geom::bbox(pts) {
            polys.push(b.as_boundary());
            approx += 1;
        }
    }
    (polys, approx)
}

/// Rectangles (used by the demo / simple callers).
fn rects_on(cell: &Cell, layer: i16, datatype: i16) -> (Vec<Rect>, usize) {
    let (polys, approx) = polys_on(cell, layer, datatype);
    (polys.iter().filter_map(|p| Rect::from_boundary(p).or_else(|| geom::bbox(p))).collect(), approx)
}

fn is_manhattan(pts: &[(i32, i32)]) -> bool {
    let n = if pts.len() >= 2 && pts.first() == pts.last() { pts.len() - 1 } else { pts.len() };
    (0..n).all(|i| {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % n];
        x1 == x2 || y1 == y2
    })
}

pub struct BoolSummary {
    pub a: usize,
    pub b: usize,
    pub out: usize,
    pub out_area: i64,
    pub approx: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn run_boolean(
    in_path: &str,
    top: Option<&str>,
    a: (i16, i16),
    b: (i16, i16),
    out_ld: (i16, i16),
    op: Op,
    out_path: &str,
) -> Result<BoolSummary, String> {
    let lib = Library::load(in_path).map_err(|e| e.to_string())?;
    let cell = pick(&lib, top)?;
    let (pa, aa) = polys_on(cell, a.0, a.1);
    let (pb, ab) = polys_on(cell, b.0, b.1);
    let (ra, rb) = (pa.len(), pb.len());
    let res = boolean::boolean_poly(&pa, &pb, op);
    let out_area = res.iter().map(|r| r.area()).sum();

    let mut oc = Cell { name: format!("{}_bool", cell.name), elements: Vec::new() };
    for r in &res {
        oc.elements.push(Element::Boundary { layer: out_ld.0, datatype: out_ld.1, pts: r.as_boundary() });
    }
    let olib = Library { name: lib.name.clone(), user_unit: lib.user_unit, db_unit: lib.db_unit, cells: vec![oc] };
    olib.save(out_path).map_err(|e| e.to_string())?;
    Ok(BoolSummary { a: ra, b: rb, out: res.len(), out_area, approx: aa + ab })
}

pub fn run_flatten(in_path: &str, top: &str, out_path: &str) -> Result<usize, String> {
    let lib = Library::load(in_path).map_err(|e| e.to_string())?;
    let flat = flatten::flatten(&lib, top)?;
    let n = flat.elements.len();
    let olib = Library { name: lib.name.clone(), user_unit: lib.user_unit, db_unit: lib.db_unit, cells: vec![flat] };
    olib.save(out_path).map_err(|e| e.to_string())?;
    Ok(n)
}

// ---- info ---------------------------------------------------------------------

fn layer_stats(lib: &Library) -> BTreeMap<(i16, i16), (usize, f64)> {
    let mut m: BTreeMap<(i16, i16), (usize, f64)> = BTreeMap::new();
    for c in &lib.cells {
        for el in &c.elements {
            if let Element::Boundary { layer, datatype, pts } = el {
                let e = m.entry((*layer, *datatype)).or_default();
                e.0 += 1;
                e.1 += geom::poly_area(pts);
            }
        }
    }
    m
}

pub fn info(lib: &Library) -> String {
    let mut s = String::new();
    s.push_str(&format!("vyges-layout — {}\n", lib.name));
    s.push_str(&format!(
        "  units     {} dbu/user, {:.3e} m/dbu\n  cells     {}\n",
        (1.0 / lib.user_unit).round() as i64,
        lib.db_unit,
        lib.cells.len()
    ));
    for c in &lib.cells {
        let mut counts = [0usize; 6]; // boundary, path, sref, aref, box, text
        for el in &c.elements {
            match el {
                Element::Boundary { .. } => counts[0] += 1,
                Element::Path { .. } => counts[1] += 1,
                Element::Sref { .. } => counts[2] += 1,
                Element::Aref { .. } => counts[3] += 1,
                Element::Box { .. } => counts[4] += 1,
                Element::Text { .. } => counts[5] += 1,
            }
        }
        s.push_str(&format!(
            "    {:<20} {} boundary · {} path · {} sref · {} aref · {} box · {} text\n",
            c.name, counts[0], counts[1], counts[2], counts[3], counts[4], counts[5]
        ));
    }
    s.push_str("  layers (layer/datatype: boundaries, area dbu²):\n");
    for ((l, d), (n, area)) in layer_stats(lib) {
        s.push_str(&format!("    {l}/{d}: {n}, {area:.0}\n"));
    }
    s
}

pub fn info_json(lib: &Library) -> String {
    let ls = layer_stats(lib);
    let n = ls.len();
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"library\": {:?},\n", lib.name));
    s.push_str(&format!("  \"db_unit_m\": {:.6e},\n", lib.db_unit));
    s.push_str(&format!("  \"cells\": {},\n", lib.cells.len()));
    s.push_str("  \"layers\": [\n");
    for (k, (&(l, d), &(cnt, area))) in ls.iter().enumerate() {
        let comma = if k + 1 < n { "," } else { "" };
        s.push_str(&format!(
            "    {{\"layer\": {l}, \"datatype\": {d}, \"boundaries\": {cnt}, \"area_dbu2\": {area:.0}}}{comma}\n"
        ));
    }
    s.push_str("  ]\n}\n");
    s
}

/// Built-in demo: two overlapping boxes on two layers, ANDed — no files.
pub fn demo() -> String {
    let lib = Library {
        name: "DEMO".into(),
        user_unit: 1e-3,
        db_unit: 1e-9,
        cells: vec![Cell {
            name: "demo".into(),
            elements: vec![
                Element::Boundary { layer: 1, datatype: 0, pts: Rect { x0: 0, y0: 0, x1: 100, y1: 100 }.as_boundary() },
                Element::Boundary { layer: 2, datatype: 0, pts: Rect { x0: 50, y0: 0, x1: 150, y1: 100 }.as_boundary() },
            ],
        }],
    };
    // round-trip through bytes, then AND the two layers
    let lib = Library::parse(&lib.to_bytes()).unwrap();
    let cell = &lib.cells[0];
    let (a, _) = rects_on(cell, 1, 0);
    let (b, _) = rects_on(cell, 2, 0);
    let and = boolean::boolean(&a, &b, Op::And);
    let area: i64 = and.iter().map(|r| r.area()).sum();
    let mut s = String::new();
    s.push_str("vyges-layout demo — two overlapping boxes, layer 1/0 AND layer 2/0\n");
    s.push_str(&format!("  GDS round-trip ok: lib {:?}, {} cell(s)\n", lib.name, lib.cells.len()));
    s.push_str(&format!("  layer 1/0: {} rect   layer 2/0: {} rect\n", a.len(), b.len()));
    s.push_str(&format!("  AND -> {} rect, area {} dbu² (expect 5000 = 50×100)\n", and.len(), area));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_runs_the_pipeline() {
        let r = demo();
        assert!(r.contains("area 5000"));
    }
}
