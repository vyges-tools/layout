//! End-to-end OASIS: the committed example survives a GDS → OASIS → GDS round-trip
//! through the public `load_any` / `save_any` API (format picked by extension).

use vyges_layout::gds::{Element, Library};
use vyges_layout::geom::{poly_area, Rect};

/// Per-layer (boundary count, total area) — the geometry fingerprint that must be
/// invariant across a format round-trip.
fn fingerprint(lib: &Library) -> std::collections::BTreeMap<(i16, i16), (usize, i64)> {
    let mut m = std::collections::BTreeMap::new();
    for c in &lib.cells {
        for e in &c.elements {
            if let Element::Boundary { layer, datatype, pts } = e {
                let ent = m.entry((*layer, *datatype)).or_insert((0usize, 0i64));
                ent.0 += 1;
                ent.1 += poly_area(pts) as i64;
            }
        }
    }
    m
}

#[test]
fn gds_to_oasis_to_gds_preserves_geometry() {
    let dir = std::env::temp_dir();
    let oas = dir.join("vyges_layout_rt.oas");
    let gds = dir.join("vyges_layout_rt.gds");
    let oas = oas.to_str().unwrap();
    let gds = gds.to_str().unwrap();

    // flatten first so hierarchy becomes plain boundaries, then compare fingerprints.
    let src = Library::load("examples/two_box.gds").expect("load gds");
    let flat = vyges_layout::flatten::flatten(&src, "top").expect("flatten");
    let flat_lib =
        Library { name: "T".into(), user_unit: src.user_unit, db_unit: src.db_unit, cells: vec![flat] };
    let want = fingerprint(&flat_lib);

    flat_lib.save_any(oas).expect("write oasis");
    let via_oasis = Library::load_any(oas).expect("read oasis");
    assert_eq!(fingerprint(&via_oasis), want, "geometry changed through OASIS");

    via_oasis.save_any(gds).expect("write gds");
    let via_gds = Library::load_any(gds).expect("read gds");
    assert_eq!(fingerprint(&via_gds), want, "geometry changed on OASIS→GDS");

    // OASIS is the compact format: it must not be larger than the GDS of the same data.
    let oas_len = std::fs::metadata(oas).unwrap().len();
    let gds_len = std::fs::metadata(gds).unwrap().len();
    assert!(oas_len <= gds_len, "oasis {oas_len} > gds {gds_len}");
}

#[test]
fn non_rectangular_polygon_survives_oasis() {
    let dir = std::env::temp_dir();
    let path = dir.join("vyges_layout_poly.oasis");
    let path = path.to_str().unwrap();

    let l_shape = vec![(0, 0), (40, 0), (40, 20), (20, 20), (20, 40), (0, 40), (0, 0)];
    let lib = Library {
        name: "P".into(),
        user_unit: 1e-3,
        db_unit: 1e-9,
        cells: vec![vyges_layout::gds::Cell {
            name: "poly".into(),
            elements: vec![Element::Boundary { layer: 5, datatype: 0, pts: l_shape.clone() }],
        }],
    };
    lib.save_any(path).unwrap();
    let back = Library::load_any(path).unwrap();
    match &back.cells[0].elements[0] {
        Element::Boundary { layer, pts, .. } => {
            assert_eq!(*layer, 5);
            assert!(Rect::from_boundary(pts).is_none(), "should stay non-rectangular");
            assert_eq!(poly_area(pts), poly_area(&l_shape));
        }
        e => panic!("expected boundary, got {e:?}"),
    }
}
