//! End-to-end: boolean kernel output (tiling rectangles) → merged oriented polygons.
//! Union a layer, then trace its boundary so edge-based checks (enclosure, spacing)
//! can measure against the *merged* edge instead of a single rectangle.

use vyges_layout::boolean::{boolean_poly, Op};
use vyges_layout::contour::trace_contours;
use vyges_layout::geom::Rect;

fn poly(r: Rect) -> Vec<(i32, i32)> {
    r.as_boundary()
}

fn area2(ring: &[(i32, i32)]) -> i64 {
    let n = ring.len() - 1;
    let mut a = 0i64;
    for i in 0..n {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[i + 1];
        a += x1 as i64 * y2 as i64 - x2 as i64 * y1 as i64;
    }
    a
}

#[test]
fn union_of_overlapping_rects_traces_to_one_ring() {
    // two overlapping metal rects OR'd: the boolean gives a rect tiling; the contour
    // is a single L-shaped (here plus-ish) merged boundary with positive area.
    let a = poly(Rect { x0: 0, y0: 0, x1: 10, y1: 10 });
    let b = poly(Rect { x0: 5, y0: 5, x1: 15, y1: 15 });
    let tiles = boolean_poly(&[a], &[b], Op::Or);
    let union_area2: i64 = tiles.iter().map(|r| 2 * r.area()).sum();

    let rings = trace_contours(&tiles);
    assert_eq!(rings.len(), 1, "the union is one connected merged polygon");
    assert!(area2(&rings[0]) > 0, "outer ring CCW");
    assert_eq!(area2(&rings[0]), union_area2, "merged boundary preserves area");
}

#[test]
fn ring_with_hole_traces_to_outer_plus_hole() {
    // a big square with a square bite out of the middle (NOT) -> annulus.
    let outer = poly(Rect { x0: 0, y0: 0, x1: 30, y1: 30 });
    let bite = poly(Rect { x0: 10, y0: 10, x1: 20, y1: 20 });
    let tiles = boolean_poly(&[outer], &[bite], Op::Not);
    let annulus_area2: i64 = tiles.iter().map(|r| 2 * r.area()).sum();

    let rings = trace_contours(&tiles);
    assert_eq!(rings.len(), 2, "outer boundary + inner hole");
    let net: i64 = rings.iter().map(|r| area2(r)).sum();
    assert_eq!(net, annulus_area2, "outer(+) plus hole(-) equals annulus area");
    assert!(rings.iter().any(|r| area2(r) > 0), "an outer CCW ring");
    assert!(rings.iter().any(|r| area2(r) < 0), "a CW hole ring");
}
