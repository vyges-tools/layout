//! Neutral netlist types produced by device extraction (`crate::extract`).
//!
//! A `Netlist` is a flat list of primitive `Device`s with a name and ordered
//! ports — the shared output of pulling devices out of a layout. It carries no
//! SPICE-parsing or comparison semantics; downstream engines (LVS comparison,
//! parasitic annotation, …) adapt it to their own domain types as needed.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub kind: char,         // normalized uppercase first letter (M/Q/R/C/L/D/X)
    pub name: String,       // instance name
    pub nodes: Vec<String>, // ordered terminal nets
    pub model: String,      // model / subckt / value (may be empty)
    /// Numeric device parameters in SI units (e.g. `w`, `l`, `nf`, `m`). Keys are
    /// lowercased.
    pub params: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default)]
pub struct Netlist {
    pub name: String,
    pub ports: Vec<String>,
    pub devices: Vec<Device>,
}

/// Render a netlist as a simple SPICE subckt (`.subckt … / <dev> <nodes> <model> / .ends`).
pub fn to_spice(nl: &Netlist) -> String {
    let mut s = String::new();
    s.push_str(&format!(".subckt {} {}\n", nl.name, nl.ports.join(" ")));
    for d in &nl.devices {
        s.push_str(&format!("{} {} {}\n", d.name, d.nodes.join(" "), d.model));
    }
    s.push_str(".ends\n");
    s
}
