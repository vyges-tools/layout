//! Oriented boundary edges and corner classification for merged rectilinear polygons.
//!
//! Consumes `contour::trace_contours` rings — where the solid interior is always on the
//! **left** of every directed edge (outer rings CCW, hole rings CW). From that it derives
//! the two primitives edge-based checks are built on:
//!
//! - **directed edges**, each carrying its axis and length (a short edge is a "tip", a
//!   long one a "side" — the distinction directional-spacing rules turn on);
//! - **corner classification**, convex vs concave *relative to the solid* — uniform across
//!   outer rings and holes because every edge already keeps the solid on its left.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// A directed boundary edge; the solid interior lies to the left of `a → b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub a: (i32, i32),
    pub b: (i32, i32),
}

impl Edge {
    pub fn axis(&self) -> Axis {
        if self.a.1 == self.b.1 {
            Axis::Horizontal
        } else {
            Axis::Vertical
        }
    }

    /// Manhattan length (edges are axis-aligned, so this is the exact length).
    pub fn len(&self) -> i64 {
        ((self.b.0 - self.a.0).abs() + (self.b.1 - self.a.1).abs()) as i64
    }

    pub fn is_empty(&self) -> bool {
        self.a == self.b
    }

    /// Unit travel direction, one of the four axis directions.
    pub fn dir(&self) -> (i32, i32) {
        ((self.b.0 - self.a.0).signum(), (self.b.1 - self.a.1).signum())
    }
}

/// A polygon vertex, classified relative to the solid interior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corner {
    pub p: (i32, i32),
    /// `true` = convex (the solid bulges out here), `false` = concave/reflex (a notch).
    pub convex: bool,
}

/// Directed edges of a closed ring (first point == last).
pub fn ring_edges(ring: &[(i32, i32)]) -> Vec<Edge> {
    ring.windows(2).map(|w| Edge { a: w[0], b: w[1] }).collect()
}

/// Corner classification for a closed ring. Because the solid is always on the left of a
/// directed edge, a left turn (positive cross product) is convex and a right turn concave
/// — the same test for outer rings and holes.
pub fn ring_corners(ring: &[(i32, i32)]) -> Vec<Corner> {
    if ring.len() < 4 {
        return Vec::new();
    }
    let pts = &ring[..ring.len() - 1]; // drop the duplicated closing point
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = pts[(i + n - 1) % n];
        let c = pts[i];
        let q = pts[(i + 1) % n];
        let din = (c.0 - p.0, c.1 - p.1);
        let dout = (q.0 - c.0, q.1 - c.1);
        let cross = din.0 as i64 * dout.1 as i64 - din.1 as i64 * dout.0 as i64;
        out.push(Corner { p: c, convex: cross > 0 });
    }
    out
}

/// Convex corners of a ring (relative to the solid) — the seed set for convex-corner rules.
pub fn convex_corners(ring: &[(i32, i32)]) -> Vec<(i32, i32)> {
    ring_corners(ring).into_iter().filter(|c| c.convex).map(|c| c.p).collect()
}

// ---- edge-set booleans -------------------------------------------------------
//
// Coincidence booleans on axis-aligned edges: `edges_and` keeps the parts of `a` that lie
// on an edge of `b` (collinear overlap), `edges_not` keeps the parts that do not. These are
// per-supporting-line 1-D interval operations — the machinery behind rules like "a via edge
// that coincides with (or departs from) the metal boundary". Direction is ignored when
// matching, but the result keeps each `a` edge's own direction.

/// Supporting line of an edge: `(axis, coord)` where axis 0 = horizontal (coord is y),
/// 1 = vertical (coord is x).
type Line = (u8, i32);

/// (supporting line, span `(lo, hi)`, `forward` = travels lo→hi). `None` for a
/// degenerate/zero-length edge.
fn line_span(e: &Edge) -> Option<(Line, (i32, i32), bool)> {
    if e.a.1 == e.b.1 && e.a.0 != e.b.0 {
        Some(((0, e.a.1), (e.a.0.min(e.b.0), e.a.0.max(e.b.0)), e.b.0 > e.a.0))
    } else if e.a.0 == e.b.0 && e.a.1 != e.b.1 {
        Some(((1, e.a.0), (e.a.1.min(e.b.1), e.a.1.max(e.b.1)), e.b.1 > e.a.1))
    } else {
        None
    }
}

/// Merge a set of 1-D closed intervals into disjoint sorted spans.
fn merge_1d(mut v: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    v.sort_unstable();
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

/// `span ∩ ⋃covers` — the parts of one span covered by the (disjoint, sorted) `covers`.
fn intersect_1d(span: (i32, i32), covers: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for &(clo, chi) in covers {
        let lo = span.0.max(clo);
        let hi = span.1.min(chi);
        if lo < hi {
            out.push((lo, hi));
        }
    }
    out
}

/// `span \ ⋃covers` — the parts of one span not covered by `covers`.
fn difference_1d(span: (i32, i32), covers: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut lo = span.0;
    for &(clo, chi) in covers {
        if chi <= lo || clo >= span.1 {
            continue;
        }
        if clo > lo {
            out.push((lo, clo));
        }
        lo = lo.max(chi);
        if lo >= span.1 {
            break;
        }
    }
    if lo < span.1 {
        out.push((lo, span.1));
    }
    out
}

/// `b`'s spans grouped and merged per supporting line.
fn cover_map(b: &[Edge]) -> std::collections::BTreeMap<Line, Vec<(i32, i32)>> {
    let mut m: std::collections::BTreeMap<Line, Vec<(i32, i32)>> = std::collections::BTreeMap::new();
    for e in b {
        if let Some((k, span, _)) = line_span(e) {
            m.entry(k).or_default().push(span);
        }
    }
    for v in m.values_mut() {
        *v = merge_1d(std::mem::take(v));
    }
    m
}

/// Rebuild oriented edges on line `key` from result spans, keeping `a`'s direction.
fn emit(key: Line, spans: &[(i32, i32)], forward: bool) -> Vec<Edge> {
    spans
        .iter()
        .map(|&(lo, hi)| {
            let (p, q) = if forward { (lo, hi) } else { (hi, lo) };
            match key.0 {
                0 => Edge { a: (p, key.1), b: (q, key.1) },
                _ => Edge { a: (key.1, p), b: (key.1, q) },
            }
        })
        .collect()
}

/// Parts of `a` that lie on (are collinear-overlapping with) an edge of `b`.
pub fn edges_and(a: &[Edge], b: &[Edge]) -> Vec<Edge> {
    let cov = cover_map(b);
    let mut out = Vec::new();
    for e in a {
        if let Some((k, span, fwd)) = line_span(e) {
            if let Some(c) = cov.get(&k) {
                out.extend(emit(k, &intersect_1d(span, c), fwd));
            }
        }
    }
    out
}

/// Parts of `a` that do **not** lie on any edge of `b`.
pub fn edges_not(a: &[Edge], b: &[Edge]) -> Vec<Edge> {
    let cov = cover_map(b);
    let mut out = Vec::new();
    for e in a {
        if let Some((k, span, fwd)) = line_span(e) {
            let spans = match cov.get(&k) {
                Some(c) => difference_1d(span, c),
                None => vec![span],
            };
            out.extend(emit(k, &spans, fwd));
        }
    }
    out
}

// ---- edge separation --------------------------------------------------------
//
// `separation` is the directional edge-to-edge spacing check: which edges of `a` face an
// edge of `b` across empty space, closer than a distance. It underlies advanced-node
// tip-to-side / tip-to-tip spacing rules once edges are classified by length.

/// Outward normal of a directed edge (right of travel; the solid is on the left, as
/// `ring_edges` produces, so the normal points into the empty space).
fn outward(e: &Edge) -> (i64, i64) {
    let dx = (e.b.0 - e.a.0).signum() as i64;
    let dy = (e.b.1 - e.a.1).signum() as i64;
    (dy, -dx)
}

/// Edge pairs `(ea in a, eb in b)` that are **parallel**, **face** each other (outward
/// normals point toward one another across empty space), **overlap in projection**, and are
/// a perpendicular gap in `(0, dist)` apart. The facing test assumes both inputs are
/// oriented with the solid on the left (as [`ring_edges`] yields). For same-set spacing
/// (`a == b`) each unordered pair is produced twice — dedupe on the caller side.
pub fn separation(a: &[Edge], b: &[Edge], dist: i64) -> Vec<(Edge, Edge)> {
    let mut out = Vec::new();
    for ea in a {
        for eb in b {
            let ha = ea.axis() == Axis::Horizontal;
            if ha != (eb.axis() == Axis::Horizontal) {
                continue; // not parallel
            }
            if ha {
                let (ya, yb) = (ea.a.1 as i64, eb.a.1 as i64);
                let gap = (ya - yb).abs();
                if gap == 0 || gap >= dist {
                    continue;
                }
                let (xa0, xa1) = (ea.a.0.min(ea.b.0), ea.a.0.max(ea.b.0));
                let (xb0, xb1) = (eb.a.0.min(eb.b.0), eb.a.0.max(eb.b.0));
                if !(xa1 >= xb0 && xb1 >= xa0) {
                    continue; // no projection overlap
                }
                if outward(ea).1 * (yb - ya) > 0 && outward(eb).1 * (ya - yb) > 0 {
                    out.push((*ea, *eb));
                }
            } else {
                let (xa, xb) = (ea.a.0 as i64, eb.a.0 as i64);
                let gap = (xa - xb).abs();
                if gap == 0 || gap >= dist {
                    continue;
                }
                let (ya0, ya1) = (ea.a.1.min(ea.b.1), ea.a.1.max(ea.b.1));
                let (yb0, yb1) = (eb.a.1.min(eb.b.1), eb.a.1.max(eb.b.1));
                if !(ya1 >= yb0 && yb1 >= ya0) {
                    continue;
                }
                if outward(ea).0 * (xb - xa) > 0 && outward(eb).0 * (xa - xb) > 0 {
                    out.push((*ea, *eb));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contour::trace_contours;
    use crate::geom::Rect;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn rectangle_four_convex_corners() {
        let rings = trace_contours(&[r(0, 0, 10, 6)]);
        let c = ring_corners(&rings[0]);
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|x| x.convex), "all rectangle corners are convex");
    }

    #[test]
    fn edges_axis_and_length() {
        let rings = trace_contours(&[r(0, 0, 10, 6)]);
        let e = ring_edges(&rings[0]);
        assert_eq!(e.len(), 4);
        let total: i64 = e.iter().map(|x| x.len()).sum();
        assert_eq!(total, 2 * (10 + 6), "perimeter");
        assert_eq!(e.iter().filter(|x| x.axis() == Axis::Horizontal).count(), 2);
        assert_eq!(e.iter().filter(|x| x.axis() == Axis::Vertical).count(), 2);
    }

    #[test]
    fn l_shape_has_one_concave_corner() {
        // vertical bar + foot: an L with exactly one reflex (concave) inner corner
        let rings = trace_contours(&[r(0, 0, 4, 12), r(4, 0, 12, 4)]);
        let c = ring_corners(&rings[0]);
        assert_eq!(c.len(), 6);
        assert_eq!(c.iter().filter(|x| x.convex).count(), 5);
        assert_eq!(c.iter().filter(|x| !x.convex).count(), 1);
    }

    #[test]
    fn hole_corners_are_concave() {
        // a square with a square hole: outer corners convex, hole corners concave
        let rects = [
            r(0, 0, 30, 10),
            r(0, 20, 30, 30),
            r(0, 10, 10, 20),
            r(20, 10, 30, 20),
        ];
        let rings = trace_contours(&rects);
        assert_eq!(rings.len(), 2);
        for ring in &rings {
            let c = ring_corners(ring);
            assert_eq!(c.len(), 4);
            // area sign tells outer (CCW, +) from hole (CW, -)
            let area2: i64 = {
                let n = ring.len() - 1;
                (0..n).map(|i| ring[i].0 as i64 * ring[i + 1].1 as i64 - ring[i + 1].0 as i64 * ring[i].1 as i64).sum()
            };
            if area2 > 0 {
                assert!(c.iter().all(|x| x.convex), "outer square: all convex");
            } else {
                assert!(c.iter().all(|x| !x.convex), "hole: all concave w.r.t. the solid");
            }
        }
    }

    fn h(x0: i32, x1: i32, y: i32) -> Edge {
        Edge { a: (x0, y), b: (x1, y) }
    }

    #[test]
    fn edges_and_keeps_the_overlap() {
        // a: x 0..20 at y=5 ; b: x 8..30 at y=5 -> overlap x 8..20
        let got = edges_and(&[h(0, 20, 5)], &[h(8, 30, 5)]);
        assert_eq!(got, vec![h(8, 20, 5)]);
    }

    #[test]
    fn edges_not_removes_the_overlap() {
        // a minus b -> the two uncovered stubs, keeping a's direction (left→right)
        let got = edges_not(&[h(0, 30, 5)], &[h(8, 20, 5)]);
        assert_eq!(got, vec![h(0, 8, 5), h(20, 30, 5)]);
    }

    #[test]
    fn perpendicular_and_offset_edges_dont_interact() {
        let vertical = Edge { a: (10, 0), b: (10, 20) };
        let other_line = h(0, 20, 9); // same orientation, different y
        let a = [h(0, 20, 5)];
        assert!(edges_and(&a, &[vertical]).is_empty());
        assert!(edges_and(&a, &[other_line]).is_empty());
        assert_eq!(edges_not(&a, &[vertical, other_line]), vec![h(0, 20, 5)]);
    }

    #[test]
    fn antiparallel_coincident_still_matches() {
        // b runs the opposite way on the same line; matching ignores direction, and the
        // result keeps a's (left→right) direction.
        let a = [h(0, 20, 5)];
        let b = [Edge { a: (18, 5), b: (2, 5) }]; // right→left, x 2..18
        assert_eq!(edges_and(&a, &b), vec![h(2, 18, 5)]);
    }

    #[test]
    fn via_edge_coincident_with_metal_boundary() {
        // the AUX.3 shape: a via's edges, split into the part on the metal boundary
        // (edges_and) and the part that departs from it (edges_not).
        let via = ring_edges(&crate::contour::trace_contours(&[Rect { x0: 0, y0: 0, x1: 10, y1: 10 }])[0]);
        // metal boundary segment coincident with the via's bottom edge, x 0..6 at y=0
        let metal = [h(0, 6, 0)];
        let on = edges_and(&via, &metal);
        let off = edges_not(&via, &metal);
        // a 6-long coincident stub on the boundary (direction follows the ring)
        assert_eq!(on.iter().map(|e| e.len()).sum::<i64>(), 6);
        // the via still has its full perimeter minus that 6-long coincident stub
        let total: i64 = via.iter().map(|e| e.len()).sum();
        let off_len: i64 = off.iter().map(|e| e.len()).sum();
        assert_eq!(off_len, total - 6);
    }

    #[test]
    fn separation_finds_only_the_facing_gap() {
        use crate::contour::trace_contours;
        let a = ring_edges(&trace_contours(&[Rect { x0: 0, y0: 0, x1: 10, y1: 10 }])[0]);
        // B is 4 dbu to the right of A (A right edge x=10, B left edge x=14)
        let b = ring_edges(&trace_contours(&[Rect { x0: 14, y0: 0, x1: 24, y1: 10 }])[0]);
        let pairs = separation(&a, &b, 5);
        assert_eq!(pairs.len(), 1, "only A's right edge faces B's left edge across the 4-gap");
        // gap == dist is not a violation
        assert!(separation(&a, &b, 4).is_empty());
        // far apart: nothing
        let c = ring_edges(&trace_contours(&[Rect { x0: 100, y0: 0, x1: 110, y1: 10 }])[0]);
        assert!(separation(&a, &c, 5).is_empty());
    }
}
