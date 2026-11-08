//! Extraction is a function of its input: the same layout and rules give the same netlist TEXT,
//! byte for byte — devices in the same order, each device's nodes in the same order, unnamed nets
//! with the same `n<id>` names. Compared raw, NOT canonicalised: a sorted comparison is exactly
//! what hid this (device and source/drain order used to follow a hash map's iteration order,
//! which is reseeded per map, so two extractions in one process already disagreed).
//!
//! Fixture: the synthetic inverter example tiled eight times side by side, labels kept only on
//! the first copy, so there are sixteen devices and many unnamed nets for an order to show in.

use vyges_layout::extract::{extract, Rules};
use vyges_layout::gds::{Element, Library};
use vyges_layout::netlist::to_spice;

const RULES: &str = "active: 1/0\npoly: 2/0\nnwell: 3/0\nconn: 1/0, 2/0, 3/0, 5/0\n\
contact: 6/0 1/0 5/0\ncontact: 6/0 2/0 5/0\ncontact: 9/0 3/0 5/0\nlabel: 8/0\n\
substrate: VSS\nnfet: nfet\npfet: pfet\n";

const COPIES: i32 = 8;

fn tiled() -> Library {
    let mut lib =
        Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/inverter.gds")).expect("example gds");
    let cell = lib.cells.iter_mut().find(|c| c.name == "inverter").expect("inverter cell");
    let xs: Vec<i32> = cell
        .elements
        .iter()
        .flat_map(|e| match e {
            Element::Boundary { pts, .. } | Element::Path { pts, .. } | Element::Box { pts, .. } => {
                pts.iter().map(|p| p.0).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect();
    let pitch = xs.iter().max().expect("geometry") - xs.iter().min().expect("geometry") + 1000;
    let base = cell.elements.clone();
    for k in 1..COPIES {
        let dx = k * pitch;
        for e in &base {
            let shift = |pts: &[(i32, i32)]| pts.iter().map(|&(x, y)| (x + dx, y)).collect::<Vec<_>>();
            match e {
                Element::Boundary { layer, datatype, pts } => {
                    cell.elements.push(Element::Boundary { layer: *layer, datatype: *datatype, pts: shift(pts) })
                }
                Element::Text { .. } => {} // copies stay unlabelled: their nets get `n<id>` names
                _ => {}
            }
        }
    }
    lib
}

#[test]
fn extraction_gives_the_same_netlist_text_every_time() {
    let rules = Rules::parse(RULES).expect("rules");
    let lib = tiled();
    let nl = extract(&lib, Some("inverter"), &rules).expect("extract");
    let first = to_spice(&nl);
    assert_eq!(nl.devices.len(), 2 * COPIES as usize, "every copy extracts its two devices:\n{first}");
    for run in 1..6 {
        let again = to_spice(&extract(&lib, Some("inverter"), &rules).expect("extract"));
        assert_eq!(again, first, "extraction {run} differs from the first");
    }
}
