//! Manhattan boolean operations on **rectilinear polygons**, via a vertical scanline.
//!
//! Each input region is reduced to its **vertical edges** with a winding sign (after
//! normalizing every polygon to CCW: a down-going edge enters the region, +1; an
//! up-going edge exits, -1). Sweeping x, the coverage of A and of B at a slab is the
//! set of y where the accumulated sign > 0 — an interval set. The op (AND/OR/NOT/XOR)
//! is applied on those interval sets and emitted as rectangles tiling the result, then
//! merged horizontally. Integer coordinates → exact; handles rectangles, L-shapes, and
//! overlapping/holey rectilinear polygons uniformly.
//!
//! Depth reserved: general-angle clipping (Vatti) and contour-tracing the output
//! rectangles back into merged polygons (the result is currently a set of rectangles).

use std::collections::{BTreeMap, BTreeSet};

use crate::geom::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    And,
    Or,
    Not, // A minus B
    Xor,
}

impl Op {
    pub fn parse(s: &str) -> Option<Op> {
        match s.to_ascii_lowercase().as_str() {
            "and" | "intersect" => Some(Op::And),
            "or" | "union" => Some(Op::Or),
            "not" | "diff" | "andnot" => Some(Op::Not),
            "xor" => Some(Op::Xor),
            _ => None,
        }
    }
}

struct VEdge {
    x: i32,
    ylo: i32,
    yhi: i32,
    sign: i32,
}

/// Vertical edges of a set of rectilinear polygons, with CCW winding signs.
fn edges(polys: &[Vec<(i32, i32)>]) -> Vec<VEdge> {
    let mut out = Vec::new();
    for poly in polys {
        let n = if poly.len() >= 2 && poly.first() == poly.last() { poly.len() - 1 } else { poly.len() };
        if n < 3 {
            continue;
        }
        let ccw = signed_area2(&poly[..n]) >= 0;
        for i in 0..n {
            let (x1, y1) = poly[i];
            let (x2, y2) = poly[(i + 1) % n];
            if x1 != x2 || y1 == y2 {
                continue; // not a vertical edge
            }
            // CCW: down-going edge (dy<0) enters (+1); flip if the polygon was CW
            let mut sign = if y2 < y1 { 1 } else { -1 };
            if !ccw {
                sign = -sign;
            }
            out.push(VEdge { x: x1, ylo: y1.min(y2), yhi: y1.max(y2), sign });
        }
    }
    out
}

/// 2× the signed polygon area (sign tells orientation; CCW > 0).
fn signed_area2(pts: &[(i32, i32)]) -> i64 {
    let mut a: i64 = 0;
    for i in 0..pts.len() {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % pts.len()];
        a += x1 as i64 * y2 as i64 - x2 as i64 * y1 as i64;
    }
    a
}

/// Signed y-coverage of one region, maintained incrementally across the x-sweep.
///
/// Keyed on y, each entry is the running `sum(sign)` delta at that y-boundary; a
/// vertical edge crossed at the current x applies `+sign` at `ylo` and `-sign` at
/// `yhi`. The region covers the y-ranges where the accumulated count is > 0.
/// Entries that net to zero are pruned so the map holds only *active* boundaries —
/// walking it to read off covered intervals is then O(active), not O(all edges).
#[derive(Default)]
struct Coverage {
    delta: std::collections::BTreeMap<i32, i32>,
}

impl Coverage {
    /// Apply one vertical edge's contribution (permanent as x moves right).
    fn apply(&mut self, ylo: i32, yhi: i32, sign: i32) {
        Self::bump(&mut self.delta, ylo, sign);
        Self::bump(&mut self.delta, yhi, -sign);
    }

    fn bump(delta: &mut std::collections::BTreeMap<i32, i32>, y: i32, d: i32) {
        let e = delta.entry(y).or_insert(0);
        *e += d;
        if *e == 0 {
            delta.remove(&y);
        }
    }

    /// Maximal y-intervals where the accumulated count is > 0.
    fn intervals(&self) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = Vec::new();
        let mut count = 0i32;
        let mut start = 0i32;
        for (&y, &d) in &self.delta {
            let prev = count;
            count += d;
            if prev <= 0 && count > 0 {
                start = y;
            } else if prev > 0 && count <= 0 && y > start {
                out.push((start, y));
            }
        }
        out
    }
}

/// Boolean on rectilinear polygons → result as rectangles tiling the region.
///
/// Active-interval plane sweep: sweep x left→right over the sorted unique edge
/// x-coords, maintaining each region's running y-coverage incrementally. At each x
/// we fold in only the edges *at that x* (O(log N) each) and read off the covered
/// intervals in O(active); the op is applied on those interval sets exactly as
/// before. O((N + K) log N) overall vs. the former O(N²) per-slab recompute.
pub fn boolean_poly(a: &[Vec<(i32, i32)>], b: &[Vec<(i32, i32)>], op: Op) -> Vec<Rect> {
    let ea = edges(a);
    let eb = edges(b);
    // Bucket each region's edges by x, and collect the sorted unique slab boundaries.
    let mut ea_by_x: BTreeMap<i32, Vec<&VEdge>> = BTreeMap::new();
    let mut eb_by_x: BTreeMap<i32, Vec<&VEdge>> = BTreeMap::new();
    let mut xs: BTreeSet<i32> = BTreeSet::new();
    for e in &ea {
        ea_by_x.entry(e.x).or_default().push(e);
        xs.insert(e.x);
    }
    for e in &eb {
        eb_by_x.entry(e.x).or_default().push(e);
        xs.insert(e.x);
    }
    let xs: Vec<i32> = xs.into_iter().collect();
    if xs.len() < 2 {
        return vec![];
    }
    let mut cov_a = Coverage::default();
    let mut cov_b = Coverage::default();
    let mut out: Vec<Rect> = Vec::new();
    for w in xs.windows(2) {
        let (xl, xr) = (w[0], w[1]);
        // Fold in every edge at xl before reading the slab [xl, xr) coverage — the
        // coverage during a slab reflects all edges with x <= xl.
        if let Some(es) = ea_by_x.get(&xl) {
            for e in es {
                cov_a.apply(e.ylo, e.yhi, e.sign);
            }
        }
        if let Some(es) = eb_by_x.get(&xl) {
            for e in es {
                cov_b.apply(e.ylo, e.yhi, e.sign);
            }
        }
        if xl == xr {
            continue;
        }
        let ia = cov_a.intervals();
        let ib = cov_b.intervals();
        let ir = match op {
            Op::And => intersect(&ia, &ib),
            Op::Or => union(&ia, &ib),
            Op::Not => difference(&ia, &ib),
            Op::Xor => union(&difference(&ia, &ib), &difference(&ib, &ia)),
        };
        for (y0, y1) in ir {
            out.push(Rect { x0: xl, y0, x1: xr, y1 });
        }
    }
    merge_horizontal(out)
}

/// Convenience wrapper for rectangle inputs.
pub fn boolean(a: &[Rect], b: &[Rect], op: Op) -> Vec<Rect> {
    let pa: Vec<_> = a.iter().map(|r| r.as_boundary()).collect();
    let pb: Vec<_> = b.iter().map(|r| r.as_boundary()).collect();
    boolean_poly(&pa, &pb, op)
}

fn union(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut v = a.to_vec();
    v.extend_from_slice(b);
    v.sort();
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (lo, hi) in v {
        if let Some(last) = out.last_mut() {
            if lo <= last.1 {
                last.1 = last.1.max(hi);
                continue;
            }
        }
        out.push((lo, hi));
    }
    out
}

fn intersect(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo < hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn difference(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for &(mut lo, hi) in a {
        for &(blo, bhi) in b {
            if bhi <= lo || blo >= hi {
                continue;
            }
            if blo > lo {
                out.push((lo, blo));
            }
            lo = lo.max(bhi);
            if lo >= hi {
                break;
            }
        }
        if lo < hi {
            out.push((lo, hi));
        }
    }
    out
}

fn merge_horizontal(mut rects: Vec<Rect>) -> Vec<Rect> {
    rects.sort_by_key(|r| (r.y0, r.y1, r.x0));
    let mut out: Vec<Rect> = Vec::new();
    for r in rects {
        if let Some(last) = out.last_mut() {
            if last.y0 == r.y0 && last.y1 == r.y1 && last.x1 == r.x0 {
                last.x1 = r.x1;
                continue;
            }
        }
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }
    fn area(v: &[Rect]) -> i64 {
        v.iter().map(|x| x.area()).sum()
    }

    #[test]
    fn or_and_not_xor_rectangles() {
        let a = [r(0, 0, 10, 10)];
        let b = [r(5, 0, 15, 10)];
        assert_eq!(area(&boolean(&a, &b, Op::Or)), 150);
        assert_eq!(boolean(&a, &b, Op::And), vec![r(5, 0, 10, 10)]);
        assert_eq!(area(&boolean(&a, &b, Op::Not)), 50);
        assert_eq!(area(&boolean(&a, &b, Op::Xor)), 100);
    }

    #[test]
    fn l_shape_polygon_intersection() {
        // L-shape (area 75) AND a box covering its lower-right notch
        let l = vec![vec![(0, 0), (10, 0), (10, 5), (5, 5), (5, 10), (0, 10), (0, 0)]];
        let box_ = vec![Rect { x0: 0, y0: 0, x1: 20, y1: 20 }.as_boundary()];
        // AND with a big covering box returns the L exactly (area 75)
        assert_eq!(area(&boolean_poly(&l, &box_, Op::And)), 75);
        // OR of the L with its missing notch (the 5x5 upper-right) fills the 10x10 square
        let notch = vec![Rect { x0: 5, y0: 5, x1: 10, y1: 10 }.as_boundary()];
        assert_eq!(area(&boolean_poly(&l, &notch, Op::Or)), 100);
    }

    #[test]
    fn difference_splits_active_by_gate() {
        // a horizontal active strip minus a vertical gate -> two source/drain regions
        let active = vec![Rect { x0: 0, y0: 0, x1: 30, y1: 10 }.as_boundary()];
        let gate = vec![Rect { x0: 13, y0: -5, x1: 17, y1: 15 }.as_boundary()];
        let sd = boolean_poly(&active, &gate, Op::Not);
        assert_eq!(sd.len(), 2, "gate splits active into source + drain");
        assert_eq!(area(&sd), 30 * 10 - 4 * 10);
    }
}
