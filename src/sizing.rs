//! Region sizing — grow / shrink merged rectilinear geometry by a margin.
//!
//! `sized(d)` is the standard DRC "bias": grow the region by `d` on every side for
//! `d > 0`, shrink it for `d < 0`. It is the primitive behind rules that build a mask
//! from a shape (e.g. an enclosure/keep-out ring) before intersecting or subtracting.
//!
//! Implementation is exact for rectilinear input via the boolean kernel:
//! - **grow** is a Minkowski sum with a `2d`-square. Since the region is the union of its
//!   tiles and the sum distributes over union, growing = the union of each tile inflated by
//!   `d`.
//! - **shrink** is the morphological dual — erosion = complement(dilate(complement)) inside
//!   a frame large enough to hold the region and its margin.

use std::cmp::Ordering;

use crate::boolean::{self, Op};
use crate::geom::Rect;

/// Bounding box of a set of rectangles (`None` if empty).
fn bbox_of(tiles: &[Rect]) -> Option<Rect> {
    let mut it = tiles.iter();
    let first = it.next()?;
    let mut b = *first;
    for r in it {
        b.x0 = b.x0.min(r.x0);
        b.y0 = b.y0.min(r.y0);
        b.x1 = b.x1.max(r.x1);
        b.y1 = b.y1.max(r.y1);
    }
    Some(b)
}

/// Grow the region (given as covering tiles) by `d > 0` on every side; the result is the
/// merged grown region as tiles. `d <= 0` just merges.
pub fn grow(tiles: &[Rect], d: i32) -> Vec<Rect> {
    if d <= 0 {
        return boolean::boolean(tiles, &[], Op::Or);
    }
    let inflated: Vec<Rect> = tiles.iter().map(|r| r.inflate(d)).collect();
    boolean::boolean(&inflated, &[], Op::Or)
}

/// Shrink the region by `d > 0` on every side (erosion). `d <= 0` just merges.
pub fn shrink(tiles: &[Rect], d: i32) -> Vec<Rect> {
    if d <= 0 {
        return boolean::boolean(tiles, &[], Op::Or);
    }
    let Some(frame) = bbox_of(tiles).map(|b| b.inflate(d + 1)) else {
        return vec![];
    };
    // erosion(R, d) = frame \ dilate(frame \ R, d)
    let comp = boolean::boolean(&[frame], tiles, Op::Not);
    let grown_comp = grow(&comp, d);
    boolean::boolean(&[frame], &grown_comp, Op::Not)
}

/// `sized(±d)`: grow for `d > 0`, shrink for `d < 0`, merge for `d == 0`.
pub fn sized(tiles: &[Rect], d: i32) -> Vec<Rect> {
    match d.cmp(&0) {
        Ordering::Greater => grow(tiles, d),
        Ordering::Less => shrink(tiles, -d),
        Ordering::Equal => boolean::boolean(tiles, &[], Op::Or),
    }
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
    fn grow_rect_all_sides() {
        let g = grow(&[r(0, 0, 100, 100)], 10);
        assert_eq!(area(&g), 120 * 120, "grown by 10 on every side");
    }

    #[test]
    fn shrink_rect_all_sides() {
        let s = shrink(&[r(0, 0, 100, 100)], 10);
        assert_eq!(area(&s), 80 * 80);
        assert_eq!(s, vec![r(10, 10, 90, 90)]);
    }

    #[test]
    fn shrink_thin_shape_to_nothing() {
        // a 15-wide bar shrunk by 10 on each side (20 total > 15) vanishes
        let s = shrink(&[r(0, 0, 15, 100)], 10);
        assert!(s.is_empty(), "over-shrink empties the region");
    }

    #[test]
    fn grow_merges_a_gap() {
        // two rects 8 apart: growing by 5 each closes the 8-gap into one region
        let g = grow(&[r(0, 0, 10, 10), r(18, 0, 28, 10)], 5);
        // one connected region spanning x -5..33, y -5..15, with no hole
        assert_eq!(g.iter().map(|t| t.x0).min().unwrap(), -5);
        assert_eq!(g.iter().map(|t| t.x1).max().unwrap(), 33);
        // fully connected: total covered width at mid-height is contiguous 38
        let covered: i64 = 38 * 20;
        assert_eq!(area(&g), covered, "gap closed, one solid region");
    }

    #[test]
    fn grow_then_shrink_restores_convex() {
        let orig = r(0, 0, 100, 60);
        let round = shrink(&grow(&[orig], 7), 7);
        assert_eq!(round, vec![orig], "open/close is identity on a convex shape");
    }

    #[test]
    fn l_shape_grow_area() {
        // L = two rects; grow by 4 and check it stays one merged region of the right area
        let l = boolean::boolean(&[r(0, 0, 40, 10), r(0, 0, 10, 40)], &[], Op::Or);
        let g = grow(&l, 4);
        // grown L bounding extent and connectivity: area equals the union of the two
        // inflated rects
        let expect = boolean::boolean(&[r(-4, -4, 44, 14), r(-4, -4, 14, 44)], &[], Op::Or);
        assert_eq!(area(&g), area(&expect));
    }
}
