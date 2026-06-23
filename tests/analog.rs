//! Domain-coverage validation: the geometry kernel has ZERO digital assumptions.
//!
//! Runs on a NON-standard-cell, analog-style layout (`examples/analog/analog.gds`:
//! a guard ring, a MIM capacitor, and a poly resistor snake — see that dir's README)
//! and proves (a) GDS write->read round-trips the geometry exactly, and (b) a polygon
//! boolean across two layers is correct. Nothing in the path knows or cares that the
//! shapes are analog rather than logic gates: layers/datatypes/polygons only.

use vyges_layout::boolean::{boolean_poly, Op};
use vyges_layout::geom::{poly_area, Rect};
use vyges_layout::gds::{Element, Library};

/// Boundary point-lists on a given layer/datatype of a cell.
fn polys(lib: &Library, cell: &str, layer: i16, dt: i16) -> Vec<Vec<(i32, i32)>> {
    let c = lib.cells.iter().find(|c| c.name == cell).unwrap();
    c.elements
        .iter()
        .filter_map(|e| match e {
            Element::Boundary { layer: l, datatype: d, pts } if *l == layer && *d == dt => {
                Some(pts.clone())
            }
            _ => None,
        })
        .collect()
}

fn area(v: &[Rect]) -> i64 {
    v.iter().map(|r| r.area()).sum()
}

/// (a) Round-trip: the analog GDS survives load -> to_bytes -> parse with every shape
/// and vertex identical. A standard-cell-free layout is not special to the kernel.
#[test]
fn analog_gds_round_trips_exactly() {
    let lib = Library::load("examples/analog/analog.gds").expect("load analog gds");
    assert_eq!(lib.cells.len(), 1);
    assert_eq!(lib.cells[0].name, "analog");
    assert_eq!(lib.cells[0].elements.len(), 5, "2 guard-ring halves + 2 cap plates + 1 snake");

    let back = Library::parse(&lib.to_bytes()).expect("re-parse");
    assert_eq!(back.name, lib.name);
    assert_eq!(back.cells.len(), lib.cells.len());

    // every boundary's exact vertices survive the round-trip
    let orig = &lib.cells[0].elements;
    let rt = &back.cells[0].elements;
    assert_eq!(orig.len(), rt.len());
    for (o, r) in orig.iter().zip(rt.iter()) {
        match (o, r) {
            (
                Element::Boundary { layer: lo, datatype: dto, pts: po },
                Element::Boundary { layer: lr, datatype: dtr, pts: pr },
            ) => {
                assert_eq!((lo, dto, po), (lr, dtr, pr), "boundary changed across round-trip");
            }
            _ => panic!("expected boundaries only"),
        }
    }
}

/// (b) Boolean across two layers: MIM-cap top plate (66/20) AND bottom plate (65/20)
/// = the exact overlap (the capacitor area); and OR of the two guard-ring halves
/// (67/20) reunites the full rectilinear ring. Pure geometry, no device notion.
#[test]
fn analog_boolean_across_layers_is_exact() {
    let lib = Library::load("examples/analog/analog.gds").expect("load analog gds");

    // cap plates overlap: bottom 300..700 x 300..700, top 350..750 x 350..650
    // -> intersection 350..700 x 350..650 = 350 * 300 = 105_000
    let bot = polys(&lib, "analog", 65, 20);
    let top = polys(&lib, "analog", 66, 20);
    let cap = boolean_poly(&top, &bot, Op::And);
    assert_eq!(cap, vec![Rect { x0: 350, y0: 350, x1: 700, y1: 650 }], "MIM cap = plate overlap");
    assert_eq!(area(&cap), 105_000);

    // guard ring: outer 0..1000, inner hole 200..800 -> ring area 1_000_000 - 360_000.
    // It is stored as a lower + upper "U"; OR reunites the full ring.
    let ring = polys(&lib, "analog", 67, 20);
    assert_eq!(ring.len(), 2, "ring stored as two halves");
    let lower = ring[0..1].to_vec();
    let upper = ring[1..2].to_vec();
    let full = boolean_poly(&lower, &upper, Op::Or);
    assert_eq!(area(&full), 640_000, "OR of the two halves = full guard ring");

    // the two halves don't overlap (the cut is at y=500): AND is empty.
    assert!(boolean_poly(&lower, &upper, Op::And).is_empty());
}

/// The resistor snake is a serpentine (non-rectangular) rectilinear polygon: it is NOT
/// an axis-aligned rectangle, yet the shoelace area is exact -- the kernel handles the
/// arbitrary analog outline directly.
#[test]
fn analog_resistor_snake_is_serpentine() {
    let lib = Library::load("examples/analog/analog.gds").expect("load analog gds");
    let snake = polys(&lib, "analog", 64, 20);
    assert_eq!(snake.len(), 1);
    let s = &snake[0];
    assert!(Rect::from_boundary(s).is_none(), "a snake is not a rectangle");
    // serpentine: three 300x100 horizontal bars (top/mid/bottom) joined by two short
    // 100-wide vertical links = 9 * 100*100 cells = 90_000 dbu^2.
    assert_eq!(poly_area(s), 90_000.0, "exact serpentine area via shoelace");
}
