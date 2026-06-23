//! Generates examples/analog/analog.gds — a NON-standard-cell, analog-style layout
//! that proves the kernel is pure geometry (no digital assumptions). The shapes are
//! what an analog designer draws, not a logic gate:
//!   - guard ring (67/20, metal1): a rectilinear ring, stored as two interlocking
//!     "U" halves whose OR reunites the full ring (for the boolean test).
//!   - MIM capacitor: a bottom plate (65/20) and an overlapping top plate (66/20) on
//!     two different layers — their AND is the capacitor area.
//!   - resistor snake (64/20, poly): a serpentine (S-shaped) rectilinear polygon.
//! Run: `cargo run --example gen_analog`.
use vyges_layout::gds::{Cell, Element, Library};
use vyges_layout::geom::Rect;

fn boundary(layer: i16, datatype: i16, pts: Vec<(i32, i32)>) -> Element {
    Element::Boundary { layer, datatype, pts }
}
fn rect(layer: i16, datatype: i16, r: Rect) -> Element {
    Element::Boundary { layer, datatype, pts: r.as_boundary() }
}

fn main() {
    // guard ring (67/20): outer 0..1000, inner hole 200..800; split by y=500 into a
    // lower "U" and an upper "U" (each a rectilinear polygon). OR -> the full ring.
    let ring_lower = vec![
        (0, 0), (1000, 0), (1000, 500), (800, 500),
        (800, 200), (200, 200), (200, 500), (0, 500), (0, 0),
    ];
    let ring_upper = vec![
        (0, 1000), (0, 500), (200, 500), (200, 800),
        (800, 800), (800, 500), (1000, 500), (1000, 1000), (0, 1000),
    ];

    // resistor snake (64/20): a serpentine S that packs length into area.
    let snake = vec![
        (300, 300), (700, 300), (700, 400), (400, 400),
        (400, 500), (700, 500), (700, 600), (300, 600), (300, 300),
    ];

    let mut lib = Library::default();
    lib.name = "ANALOG".into();
    lib.cells.push(Cell {
        name: "analog".into(),
        elements: vec![
            boundary(67, 20, ring_lower),                       // guard ring (lower U)
            boundary(67, 20, ring_upper),                       // guard ring (upper U)
            rect(65, 20, Rect::new(300, 300, 700, 700)),        // MIM cap bottom plate
            rect(66, 20, Rect::new(350, 350, 750, 650)),        // MIM cap top plate
            boundary(64, 20, snake),                            // poly resistor snake
        ],
    });
    lib.save("examples/analog/analog.gds").unwrap();
    println!("wrote examples/analog/analog.gds ({} cells)", lib.cells.len());
}
