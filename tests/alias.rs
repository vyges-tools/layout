//! A multi-patterned metal drawn on one GDS datatype per mask is one conductor: the extraction
//! rules' `alias:` reads each mask datatype as the layer's plain one. The inverter example
//! (`tests/data/inverter.gds`, synthetic) with every metal1 (5/0) boundary split into two halves,
//! the second half moved to 5/1 (a second mask), extracts the same netlist as the uncoloured
//! layout with `alias: 5/1 5/0`, and a different one without it.

use vyges_layout::extract::{extract, Rules};
use vyges_layout::gds::{Element, Library};
use vyges_layout::netlist::Netlist;

const RULES: &str = "active: 1/0\npoly: 2/0\nnwell: 3/0\nconn: 1/0, 2/0, 3/0, 5/0\n\
contact: 6/0 1/0 5/0\ncontact: 6/0 2/0 5/0\ncontact: 9/0 3/0 5/0\nlabel: 8/0\n\
substrate: VSS\nnfet: nfet\npfet: pfet\n";

fn lib() -> Library {
    Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/inverter.gds")).expect("example gds")
}

/// Every 5/0 boundary split across its longer side; the far half goes to 5/1.
fn coloured(mut lib: Library) -> (Library, usize) {
    let mut split = 0;
    for c in &mut lib.cells {
        let mut out = Vec::new();
        for e in c.elements.drain(..) {
            let Element::Boundary { layer: 5, datatype: 0, ref pts } = e else {
                out.push(e);
                continue;
            };
            let (x0, x1) = (pts.iter().map(|p| p.0).min().unwrap(), pts.iter().map(|p| p.0).max().unwrap());
            let (y0, y1) = (pts.iter().map(|p| p.1).min().unwrap(), pts.iter().map(|p| p.1).max().unwrap());
            if pts.len() != 5 {
                out.push(e);
                continue;
            }
            let r = |a: i32, b: i32, c: i32, d: i32| vec![(a, b), (c, b), (c, d), (a, d), (a, b)];
            let (h0, h1) = if x1 - x0 >= y1 - y0 {
                let m = (x0 + x1) / 2;
                (r(x0, y0, m, y1), r(m, y0, x1, y1))
            } else {
                let m = (y0 + y1) / 2;
                (r(x0, y0, x1, m), r(x0, m, x1, y1))
            };
            out.push(Element::Boundary { layer: 5, datatype: 0, pts: h0 });
            out.push(Element::Boundary { layer: 5, datatype: 1, pts: h1 });
            split += 1;
        }
        c.elements = out;
    }
    (lib, split)
}

/// The netlist as LVS compares it: ports, then the devices as a sorted list, each keyed by model,
/// gate, bulk and its {source, drain} pair unordered. Two DIFFERENT layouts of one circuit (here:
/// metal split across datatypes) feed the extractor shapes in a different order, so their device
/// order and source/drain order differ; neither is part of the circuit. (One layout always gives
/// the same text: `tests/determinism.rs`.)
fn key(n: &Netlist) -> String {
    let mut devs: Vec<(String, String, String, [String; 2])> = n
        .devices
        .iter()
        .map(|d| {
            let mut sd = [d.nodes[0].clone(), d.nodes[2].clone()];
            sd.sort();
            (d.model.clone(), d.nodes[1].clone(), d.nodes[3].clone(), sd)
        })
        .collect();
    devs.sort();
    format!("{:?} {devs:?}", n.ports)
}

#[test]
fn an_alias_merges_mask_datatypes_into_one_conductor() {
    let rules = Rules::parse(RULES).expect("rules");
    let plain = extract(&lib(), Some("inverter"), &rules).expect("extract plain");
    let (col, split) = coloured(lib());
    assert!(split >= 2, "the example has metal1 to colour ({split})");
    let without = extract(&col, Some("inverter"), &rules).expect("extract coloured");
    assert_ne!(key(&without), key(&plain), "without the alias the second mask is not a conductor");
    let aliased = Rules::parse(&format!("{RULES}alias: 5/1 5/0\n")).expect("rules with alias");
    let with = extract(&col, Some("inverter"), &aliased).expect("extract aliased");
    assert_eq!(key(&with), key(&plain));
}

#[test]
fn a_malformed_alias_is_an_error() {
    assert!(Rules::parse(&format!("{RULES}alias: 5/1\n")).is_err());
    assert!(Rules::parse(&format!("{RULES}alias: 5/x 5/0\n")).is_err());
}

/// A label on a mask's own text datatype (8/1) names its net once aliased to the label layer.
#[test]
fn an_alias_moves_labels_too() {
    let rules = Rules::parse(RULES).expect("rules");
    let plain = extract(&lib(), Some("inverter"), &rules).expect("extract plain");
    let mut l = lib();
    let mut moved = 0;
    for c in &mut l.cells {
        for e in &mut c.elements {
            if let Element::Text { layer: 8, texttype, .. } = e {
                *texttype = 1;
                moved += 1;
            }
        }
    }
    assert!(moved >= 4, "the example's four port labels ({moved})");
    let without = extract(&l, Some("inverter"), &rules).expect("extract");
    assert_ne!(without.ports, plain.ports);
    let aliased = Rules::parse(&format!("{RULES}alias: 8/1 8/0\n")).expect("rules with alias");
    assert_eq!(key(&extract(&l, Some("inverter"), &aliased).expect("extract aliased")), key(&plain));
}
