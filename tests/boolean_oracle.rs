//! Independent-reference oracle for the scanline boolean.
//!
//! The boolean op is computed two ways and cross-checked: the production **scanline**
//! (`boolean::boolean`) vs. a dead-simple **rasterizer** that decides set membership
//! per unit pixel. Different algorithms, same answer required — so neither is its own
//! reference (the "no single path validates itself" discipline).

use vyges_layout::boolean::{boolean, Op};
use vyges_layout::geom::Rect;

/// Reference area: count unit pixels whose centre falls in the op's result set.
/// Pixels are `[x, x+1) × [y, y+1)`, tested at their `(x+0.5, y+0.5)` centre, so the
/// count equals the exact integer area of an axis-aligned rectilinear region.
fn raster_area(a: &[Rect], b: &[Rect], op: Op, extent: Rect) -> i64 {
    let inside = |rs: &[Rect], x: i32, y: i32| rs.iter().any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1);
    let mut area = 0i64;
    for y in extent.y0..extent.y1 {
        for x in extent.x0..extent.x1 {
            let ina = inside(a, x, y);
            let inb = inside(b, x, y);
            let hit = match op {
                Op::And => ina && inb,
                Op::Or => ina || inb,
                Op::Not => ina && !inb,
                Op::Xor => ina ^ inb,
            };
            if hit {
                area += 1;
            }
        }
    }
    area
}

#[test]
fn scanline_boolean_matches_the_rasterizer() {
    // A few overlapping rectangles per operand (holes, L-shapes emerge from the ops).
    let a = vec![Rect::new(0, 0, 60, 40), Rect::new(20, 30, 90, 80), Rect::new(70, 0, 100, 50)];
    let b = vec![Rect::new(30, 10, 80, 70), Rect::new(0, 50, 40, 100)];
    let extent = Rect::new(0, 0, 100, 100);

    for op in [Op::And, Op::Or, Op::Not, Op::Xor] {
        let got: i64 = boolean(&a, &b, op).iter().map(|r| r.area()).sum();
        let want = raster_area(&a, &b, op, extent);
        assert_eq!(got, want, "{op:?}: scanline area {got} != rasterized area {want}");
    }
}

#[test]
fn boolean_edge_cases_match() {
    let extent = Rect::new(-10, -10, 60, 60);
    let cases: &[(Vec<Rect>, Vec<Rect>)] = &[
        (vec![], vec![Rect::new(0, 0, 10, 10)]),                              // empty A
        (vec![Rect::new(0, 0, 10, 10)], vec![]),                             // empty B
        (vec![Rect::new(0, 0, 20, 20)], vec![Rect::new(0, 0, 20, 20)]),      // identical
        (vec![Rect::new(0, 0, 20, 20)], vec![Rect::new(20, 0, 40, 20)]),     // abutting (disjoint)
        (vec![Rect::new(0, 0, 30, 30)], vec![Rect::new(10, 10, 20, 20)]),    // B inside A (hole)
    ];
    for (a, b) in cases {
        for op in [Op::And, Op::Or, Op::Not, Op::Xor] {
            let got: i64 = boolean(a, b, op).iter().map(|r| r.area()).sum();
            let want = raster_area(a, b, op, extent);
            assert_eq!(got, want, "{op:?} on {a:?} vs {b:?}");
        }
    }
}
