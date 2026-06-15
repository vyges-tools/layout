//! Generates examples/two_box.gds — a tiny layout used by the CLI examples:
//! a `top` cell with two overlapping metal boxes (for boolean) + two placements of
//! a `tile` leaf cell (for flatten). Run: `cargo run --example gen`.
use vyges_layout::gds::{Cell, Element, Library};
use vyges_layout::geom::Rect;

fn boundary(layer: i16, datatype: i16, r: Rect) -> Element {
    Element::Boundary { layer, datatype, pts: r.as_boundary() }
}

fn main() {
    let mut lib = Library::default();
    lib.name = "EXAMPLE".into();
    lib.cells.push(Cell {
        name: "tile".into(),
        elements: vec![boundary(68, 20, Rect::new(0, 0, 200, 200))],
    });
    lib.cells.push(Cell {
        name: "top".into(),
        elements: vec![
            boundary(67, 20, Rect::new(0, 0, 1000, 500)),     // met1
            boundary(68, 20, Rect::new(400, 0, 1400, 500)),   // met2 (overlaps met1)
            Element::Sref { sname: "tile".into(), x: 0, y: 600, reflect: false, mag: 1.0, angle: 0.0 },
            Element::Sref { sname: "tile".into(), x: 300, y: 600, reflect: false, mag: 1.0, angle: 0.0 },
        ],
    });
    lib.save("examples/two_box.gds").unwrap();
    println!("wrote examples/two_box.gds ({} cells)", lib.cells.len());
}
