//! End-to-end on the committed example GDS: read, boolean, flatten.

use vyges_layout::boolean::{boolean, Op};
use vyges_layout::flatten::flatten;
use vyges_layout::gds::{Element, Library};
use vyges_layout::geom::Rect;

fn rects(lib: &Library, cell: &str, layer: i16, dt: i16) -> Vec<Rect> {
    let c = lib.cells.iter().find(|c| c.name == cell).unwrap();
    c.elements
        .iter()
        .filter_map(|e| match e {
            Element::Boundary { layer: l, datatype: d, pts } if *l == layer && *d == dt => {
                Rect::from_boundary(pts)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn read_boolean_flatten_the_example() {
    let lib = Library::load("examples/two_box.gds").expect("load gds");
    assert_eq!(lib.cells.len(), 2);

    // met1 (67/20) AND met2 (68/20) on `top` = 600x500 overlap = 300000 dbu^2
    let a = rects(&lib, "top", 67, 20);
    let b = rects(&lib, "top", 68, 20);
    let and = boolean(&a, &b, Op::And);
    assert_eq!(and.iter().map(|r| r.area()).sum::<i64>(), 300_000);
    assert_eq!(and, vec![Rect { x0: 400, y0: 0, x1: 1000, y1: 500 }]);

    // flatten expands the two SREFs of `tile` -> 4 boundaries total
    let flat = flatten(&lib, "top").expect("flatten");
    assert_eq!(flat.elements.len(), 4);
}

#[test]
fn gds_round_trips_through_bytes() {
    let lib = Library::load("examples/two_box.gds").unwrap();
    let back = Library::parse(&lib.to_bytes()).unwrap();
    assert_eq!(back.name, lib.name);
    assert_eq!(back.cells.len(), lib.cells.len());
}
