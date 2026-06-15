//! Hierarchy flatten — expand SREF/AREF references into a single cell's coordinate
//! space. Each reference applies reflection-about-x → magnification → rotation, then
//! translation (the GDS order), composed down the hierarchy. AREF arrays expand over
//! their col/row pitch. Cycles are guarded; arbitrary angles round to integer DBU.

use std::collections::HashMap;

use crate::gds::{Cell, Element, Library};

#[derive(Clone, Copy)]
struct Tf {
    m: [[f64; 2]; 2],
    t: (f64, f64),
}

impl Tf {
    fn id() -> Tf {
        Tf { m: [[1.0, 0.0], [0.0, 1.0]], t: (0.0, 0.0) }
    }
    fn apply(&self, p: (i32, i32)) -> (i32, i32) {
        let (x, y) = (p.0 as f64, p.1 as f64);
        (
            (self.m[0][0] * x + self.m[0][1] * y + self.t.0).round() as i32,
            (self.m[1][0] * x + self.m[1][1] * y + self.t.1).round() as i32,
        )
    }
    /// self ∘ inner  (apply inner first, then self)
    fn then(&self, inner: &Tf) -> Tf {
        let m = [
            [
                self.m[0][0] * inner.m[0][0] + self.m[0][1] * inner.m[1][0],
                self.m[0][0] * inner.m[0][1] + self.m[0][1] * inner.m[1][1],
            ],
            [
                self.m[1][0] * inner.m[0][0] + self.m[1][1] * inner.m[1][0],
                self.m[1][0] * inner.m[0][1] + self.m[1][1] * inner.m[1][1],
            ],
        ];
        let t = (
            self.m[0][0] * inner.t.0 + self.m[0][1] * inner.t.1 + self.t.0,
            self.m[1][0] * inner.t.0 + self.m[1][1] * inner.t.1 + self.t.1,
        );
        Tf { m, t }
    }
}

/// Ref transform from strans (reflect-x → mag → rotate) + origin translation.
fn ref_tf(x: i32, y: i32, reflect: bool, mag: f64, angle_deg: f64) -> Tf {
    let mag = if mag <= 0.0 { 1.0 } else { mag };
    let th = angle_deg.to_radians();
    let (c, s) = (th.cos(), th.sin());
    // rotate * mag * reflect-x
    let ry = if reflect { -1.0 } else { 1.0 };
    let m = [[c * mag, -s * mag * ry], [s * mag, c * mag * ry]];
    Tf { m, t: (x as f64, y as f64) }
}

pub fn flatten(lib: &Library, top: &str) -> Result<Cell, String> {
    let map: HashMap<&str, &Cell> =
        lib.cells.iter().map(|c| (c.name.as_str(), c)).collect();
    if !map.contains_key(top) {
        return Err(format!("cell {top:?} not found"));
    }
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    expand(top, &map, Tf::id(), &mut out, &mut stack)?;
    Ok(Cell { name: format!("{top}_flat"), elements: out })
}

fn expand(
    name: &str,
    map: &HashMap<&str, &Cell>,
    tf: Tf,
    out: &mut Vec<Element>,
    stack: &mut Vec<String>,
) -> Result<(), String> {
    if stack.len() > 64 {
        return Err("hierarchy too deep (cycle?)".into());
    }
    if stack.iter().any(|n| n == name) {
        return Ok(()); // cycle — skip
    }
    let Some(cell) = map.get(name) else {
        return Ok(()); // missing ref — skip (don't fail the whole flatten)
    };
    stack.push(name.to_string());
    for el in &cell.elements {
        match el {
            Element::Boundary { layer, datatype, pts } => out.push(Element::Boundary {
                layer: *layer,
                datatype: *datatype,
                pts: pts.iter().map(|&p| tf.apply(p)).collect(),
            }),
            Element::Path { layer, datatype, width, pts } => out.push(Element::Path {
                layer: *layer,
                datatype: *datatype,
                width: *width,
                pts: pts.iter().map(|&p| tf.apply(p)).collect(),
            }),
            Element::Box { layer, boxtype, pts } => out.push(Element::Box {
                layer: *layer,
                boxtype: *boxtype,
                pts: pts.iter().map(|&p| tf.apply(p)).collect(),
            }),
            Element::Text { layer, texttype, x, y, string } => {
                let (nx, ny) = tf.apply((*x, *y));
                out.push(Element::Text { layer: *layer, texttype: *texttype, x: nx, y: ny, string: string.clone() });
            }
            Element::Sref { sname, x, y, reflect, mag, angle } => {
                let child = tf.then(&ref_tf(*x, *y, *reflect, *mag, *angle));
                expand(sname, map, child, out, stack)?;
            }
            Element::Aref { sname, cols, rows, pts, reflect, mag, angle } => {
                if pts.len() >= 3 {
                    let (cols, rows) = ((*cols).max(1) as i32, (*rows).max(1) as i32);
                    let o = pts[0];
                    let colstep = ((pts[1].0 - o.0) / cols, (pts[1].1 - o.1) / cols);
                    let rowstep = ((pts[2].0 - o.0) / rows, (pts[2].1 - o.1) / rows);
                    for cc in 0..cols {
                        for rr in 0..rows {
                            let ox = o.0 + cc * colstep.0 + rr * rowstep.0;
                            let oy = o.1 + cc * colstep.1 + rr * rowstep.1;
                            let child = tf.then(&ref_tf(ox, oy, *reflect, *mag, *angle));
                            expand(sname, map, child, out, stack)?;
                        }
                    }
                }
            }
        }
    }
    stack.pop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gds::{Cell, Library};

    #[test]
    fn sref_translates_child() {
        let mut lib = Library::default();
        lib.cells.push(Cell {
            name: "leaf".into(),
            elements: vec![Element::Boundary {
                layer: 1,
                datatype: 0,
                pts: vec![(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)],
            }],
        });
        lib.cells.push(Cell {
            name: "top".into(),
            elements: vec![
                Element::Sref { sname: "leaf".into(), x: 0, y: 0, reflect: false, mag: 1.0, angle: 0.0 },
                Element::Sref { sname: "leaf".into(), x: 100, y: 0, reflect: false, mag: 1.0, angle: 0.0 },
            ],
        });
        let flat = flatten(&lib, "top").unwrap();
        assert_eq!(flat.elements.len(), 2);
        // second instance shifted by +100 in x
        if let Element::Boundary { pts, .. } = &flat.elements[1] {
            assert_eq!(pts[1], (110, 0));
        } else {
            panic!();
        }
    }
}
