//! Contour tracing: a set of tiling rectangles (e.g. `boolean::boolean_poly` output)
//! → merged, oriented rectilinear polygons.
//!
//! The boolean kernel returns the result as *rectangles tiling the region*. Edge-based
//! checks — via enclosure, spacing, and similar rules — instead need the **merged
//! boundary**, the oriented edges of the union. This module recovers that: outer rings
//! come out CCW (signed area > 0), holes CW (< 0), matching `boolean::signed_area2`.
//!
//! Method (all integer, exact for rectilinear input):
//! 1. Reduce every rectangle to signed edge intervals — a left edge contributes
//!    interior-to-the-right, a right edge interior-to-the-left; abutting opposite
//!    edges cancel. A signed sweep per column (per row) yields the union's boundary
//!    walls as **directed** maximal segments (interior on the left of travel).
//! 2. Split those segments at every shared vertex and stitch head-to-tail into
//!    closed rings; at a point-touch junction the sharpest-clockwise turn keeps the
//!    two rings separate rather than fusing them into a figure-eight.
//!
//! Input rectangles may abut or share partial edges but must not overlap in area
//! (the `boolean_poly` output guarantees this).

use std::collections::HashMap;

use crate::geom::Rect;

/// Trace the boundary of the union of `rects` into closed, oriented rings.
/// Each ring is closed (first point == last). Outer rings are CCW, holes CW.
pub fn trace_contours(rects: &[Rect]) -> Vec<Vec<(i32, i32)>> {
    // --- 1. boundary walls as directed maximal segments -----------------------
    // Vertical walls: group left(+1)/right(-1) edges by x.
    let mut vcol: HashMap<i32, Vec<(i32, i32, i32)>> = HashMap::new(); // x -> [(ylo,yhi,sign)]
    // Horizontal walls: group bottom(+1)/top(-1) edges by y.
    let mut hrow: HashMap<i32, Vec<(i32, i32, i32)>> = HashMap::new(); // y -> [(xlo,xhi,sign)]
    for r in rects {
        if r.x0 == r.x1 || r.y0 == r.y1 {
            continue;
        }
        vcol.entry(r.x0).or_default().push((r.y0, r.y1, 1)); // left edge
        vcol.entry(r.x1).or_default().push((r.y0, r.y1, -1)); // right edge
        hrow.entry(r.y0).or_default().push((r.x0, r.x1, 1)); // bottom edge
        hrow.entry(r.y1).or_default().push((r.x0, r.x1, -1)); // top edge
    }

    let mut edges: Vec<[(i32, i32); 2]> = Vec::new(); // directed [from, to]
    for (&x, ivals) in &vcol {
        for (lo, hi, net) in signed_runs(ivals) {
            match net {
                // union left wall: interior to the east → travel south (interior on left)
                1 => edges.push([(x, hi), (x, lo)]),
                // union right wall: interior to the west → travel north
                -1 => edges.push([(x, lo), (x, hi)]),
                _ => {} // |net| != 1 can't happen for non-overlapping input
            }
        }
    }
    for (&y, ivals) in &hrow {
        for (lo, hi, net) in signed_runs(ivals) {
            match net {
                // union bottom wall: interior to the north → travel east (interior on left)
                1 => edges.push([(lo, y), (hi, y)]),
                // union top wall: interior to the south → travel west
                -1 => edges.push([(hi, y), (lo, y)]),
                _ => {}
            }
        }
    }

    // --- 2. split every segment at all vertices, then stitch ------------------
    let mut verts: Vec<(i32, i32)> = edges.iter().flat_map(|e| [e[0], e[1]]).collect();
    verts.sort_unstable();
    verts.dedup();

    let mut units: Vec<[(i32, i32); 2]> = Vec::new();
    for e in &edges {
        let (a, b) = (e[0], e[1]);
        if a.0 == b.0 {
            // vertical: collect vertices on this column between a.y and b.y
            let x = a.0;
            let (lo, hi) = (a.1.min(b.1), a.1.max(b.1));
            let mut ys: Vec<i32> =
                verts.iter().filter(|&&(vx, vy)| vx == x && vy >= lo && vy <= hi).map(|&(_, vy)| vy).collect();
            ys.sort_unstable();
            ys.dedup();
            let going_up = b.1 > a.1;
            for w in ys.windows(2) {
                if going_up {
                    units.push([(x, w[0]), (x, w[1])]);
                } else {
                    units.push([(x, w[1]), (x, w[0])]);
                }
            }
        } else {
            let y = a.1;
            let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
            let mut xs: Vec<i32> =
                verts.iter().filter(|&&(vx, vy)| vy == y && vx >= lo && vx <= hi).map(|&(vx, _)| vx).collect();
            xs.sort_unstable();
            xs.dedup();
            let going_right = b.0 > a.0;
            for w in xs.windows(2) {
                if going_right {
                    units.push([(w[0], y), (w[1], y)]);
                } else {
                    units.push([(w[1], y), (w[0], y)]);
                }
            }
        }
    }

    // adjacency: start point -> unit-edge indices
    let mut out_of: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, u) in units.iter().enumerate() {
        out_of.entry(u[0]).or_default().push(i);
    }
    let mut used = vec![false; units.len()];

    let mut rings: Vec<Vec<(i32, i32)>> = Vec::new();
    for start in 0..units.len() {
        if used[start] {
            continue;
        }
        let mut ring: Vec<(i32, i32)> = Vec::new();
        let mut ei = start;
        loop {
            used[ei] = true;
            let e = units[ei];
            ring.push(e[0]);
            let din = dir(e[0], e[1]);
            // pick the sharpest-clockwise unused outgoing edge at e[1]
            let cands = out_of.get(&e[1]);
            let mut next: Option<usize> = None;
            let mut best = 5;
            if let Some(cands) = cands {
                for &c in cands {
                    if used[c] {
                        continue;
                    }
                    let rank = turn_rank(din, dir(units[c][0], units[c][1]));
                    if rank < best {
                        best = rank;
                        next = Some(c);
                    }
                }
            }
            match next {
                Some(n) => ei = n,
                None => break, // ring closes back at `start`
            }
        }
        if let Some(&first) = ring.first() {
            ring.push(first); // close
        }
        let ring = simplify(ring);
        if ring.len() >= 4 {
            rings.push(ring);
        }
    }
    rings
}

/// Maximal runs of constant nonzero signed coverage over a set of signed intervals.
/// Returns `(lo, hi, net)` with adjacent equal-`net` runs merged.
fn signed_runs(ivals: &[(i32, i32, i32)]) -> Vec<(i32, i32, i32)> {
    let mut ev: Vec<(i32, i32)> = Vec::with_capacity(ivals.len() * 2);
    for &(lo, hi, s) in ivals {
        ev.push((lo, s));
        ev.push((hi, -s));
    }
    ev.sort_unstable();
    let mut raw: Vec<(i32, i32, i32)> = Vec::new();
    let mut acc = 0i32;
    let mut seg_start = 0i32;
    let mut i = 0;
    while i < ev.len() {
        let c = ev[i].0;
        if acc != 0 {
            raw.push((seg_start, c, acc));
        }
        while i < ev.len() && ev[i].0 == c {
            acc += ev[i].1;
            i += 1;
        }
        seg_start = c;
    }
    // merge adjacent runs with the same net value
    let mut out: Vec<(i32, i32, i32)> = Vec::new();
    for (lo, hi, net) in raw {
        if let Some(last) = out.last_mut() {
            if last.1 == lo && last.2 == net {
                last.1 = hi;
                continue;
            }
        }
        out.push((lo, hi, net));
    }
    out
}

fn dir(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    ((b.0 - a.0).signum(), (b.1 - a.1).signum())
}

/// Preference for the outgoing direction relative to the incoming one, tracing with
/// interior on the left: left turn (sharpest counterclockwise) first, then straight,
/// then right, then a reversal last. At a point-touch junction this keeps a shape
/// hugging its own interior instead of fusing with the shape it merely touches.
/// Lower is preferred.
fn turn_rank(din: (i32, i32), dout: (i32, i32)) -> i32 {
    let right = (din.1, -din.0);
    let left = (-din.1, din.0);
    let rev = (-din.0, -din.1);
    if dout == left {
        0
    } else if dout == din {
        1
    } else if dout == right {
        2
    } else if dout == rev {
        3
    } else {
        4
    }
}

/// Drop collinear interior vertices from a closed ring.
fn simplify(ring: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    if ring.len() < 4 {
        return ring;
    }
    // work on the open sequence (drop the duplicated closing point)
    let pts = &ring[..ring.len() - 1];
    let n = pts.len();
    let mut keep: Vec<(i32, i32)> = Vec::with_capacity(n);
    for i in 0..n {
        let p = pts[(i + n - 1) % n];
        let c = pts[i];
        let q = pts[(i + 1) % n];
        // collinear if the cross product of (c-p) and (q-c) is zero
        let cross = (c.0 - p.0) as i64 * (q.1 - c.1) as i64 - (c.1 - p.1) as i64 * (q.0 - c.0) as i64;
        if cross != 0 {
            keep.push(c);
        }
    }
    if let Some(&first) = keep.first() {
        keep.push(first);
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    /// 2× signed area of a closed ring (CCW > 0).
    fn area2(ring: &[(i32, i32)]) -> i64 {
        let n = ring.len() - 1; // last == first
        let mut a = 0i64;
        for i in 0..n {
            let (x1, y1) = ring[i];
            let (x2, y2) = ring[i + 1];
            a += x1 as i64 * y2 as i64 - x2 as i64 * y1 as i64;
        }
        a
    }

    fn net_area2(rings: &[Vec<(i32, i32)>]) -> i64 {
        rings.iter().map(|r| area2(r)).sum()
    }

    fn input_area2(rects: &[Rect]) -> i64 {
        rects.iter().map(|r| 2 * r.area()).sum()
    }

    #[test]
    fn single_rect_one_ccw_ring() {
        let rects = [r(0, 0, 10, 6)];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 5, "4 corners + close");
        assert!(area2(&rings[0]) > 0, "outer ring is CCW");
        assert_eq!(net_area2(&rings), input_area2(&rects));
    }

    #[test]
    fn two_abutting_rects_merge_to_one_ring() {
        // share the edge x=10 fully -> a single 20x6 rectangle, 4 corners
        let rects = [r(0, 0, 10, 6), r(10, 0, 20, 6)];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 5, "collinear join dropped");
        assert_eq!(net_area2(&rings), input_area2(&rects));
    }

    #[test]
    fn l_shape_from_two_rects() {
        // vertical bar + horizontal foot sharing a partial edge (T-junction)
        let rects = [r(0, 0, 4, 12), r(4, 0, 12, 4)];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 7, "6 corners + close");
        assert!(area2(&rings[0]) > 0);
        assert_eq!(net_area2(&rings), input_area2(&rects));
    }

    #[test]
    fn donut_outer_ccw_hole_cw() {
        // a ring of rects around a central hole (a 30x30 square minus its 10x10 core)
        let rects = [
            r(0, 0, 30, 10),  // bottom band
            r(0, 20, 30, 30), // top band
            r(0, 10, 10, 20), // left band
            r(20, 10, 30, 20), // right band
        ];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 2, "outer boundary + hole");
        let (mut outer, mut hole) = (0i64, 0i64);
        for ring in &rings {
            let a = area2(ring);
            if a > 0 {
                outer = a;
            } else {
                hole = a;
            }
        }
        assert!(outer > 0 && hole < 0, "outer CCW, hole CW");
        assert_eq!(outer + hole, input_area2(&rects), "annulus area preserved");
        assert_eq!(outer, 2 * 30 * 30);
        assert_eq!(hole, -2 * 10 * 10);
    }

    #[test]
    fn corner_touch_gives_two_rings() {
        // two squares meeting only at the point (10,10): must not fuse
        let rects = [r(0, 0, 10, 10), r(10, 10, 20, 20)];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 2, "point-touch stays two rings");
        for ring in &rings {
            assert_eq!(ring.len(), 5);
            assert!(area2(ring) > 0);
        }
        assert_eq!(net_area2(&rings), input_area2(&rects));
    }

    #[test]
    fn staircase_area_preserved() {
        // an irregular staircase of stacked rects of varying width
        let rects = [
            r(0, 0, 30, 5),
            r(0, 5, 22, 10),
            r(0, 10, 14, 15),
            r(0, 15, 6, 20),
        ];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 1);
        assert_eq!(net_area2(&rings), input_area2(&rects));
        assert!(area2(&rings[0]) > 0);
    }
}
