//! Scale + differential guards for the plane-sweep boolean and contour kernels.
//!
//! Two things are asserted here that the small unit oracles can't:
//!   1. **Correctness at volume** — the plane-sweep `boolean` is diffed against the
//!      independent unit-pixel rasterizer on many randomized inputs, across all ops,
//!      and `trace_contours` is diffed against the boolean's own area.
//!   2. **Complexity** — a 10^5-shape layout completes well within a wall-clock bound.
//!      The former O(N^2) kernels needed minutes at this size, so a regression back to
//!      quadratic trips these bounds. The bounds are deliberately loose (seconds, not
//!      the ~0.1s we actually see) so they don't flake on a slow CI runner while still
//!      catching a return to O(N^2).

use std::time::Instant;

use vyges_layout::boolean::{boolean, Op};
use vyges_layout::contour::trace_contours;
use vyges_layout::geom::Rect;

/// Reference area: count unit pixels whose centre falls in the op's result set.
/// (Same construction as `boolean_oracle.rs`, kept local so this file is standalone.)
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

/// Deterministic LCG so the randomized cases are reproducible across runs/platforms.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() >> 33) as i32 % (hi - lo)
    }
    fn rects(&mut self, n: usize, bound: i32, max_w: i32) -> Vec<Rect> {
        (0..n)
            .map(|_| {
                let x0 = self.range(0, bound);
                let y0 = self.range(0, bound);
                Rect::new(x0, y0, x0 + self.range(1, max_w + 1), y0 + self.range(1, max_w + 1))
            })
            .collect()
    }
}

#[test]
fn plane_sweep_boolean_matches_rasterizer_on_random_inputs() {
    let mut rng = Lcg(0x9E3779B97F4A7C15);
    // Extent must cover the full generation range (max coord = bound + max_w) so the
    // rasterizer counts every pixel the plane-sweep can emit.
    let extent = Rect::new(0, 0, 70, 70);
    for trial in 0..200 {
        // Overlapping rects within the extent so holes/L-shapes/islands emerge.
        let (na, nb) = (rng.range(0, 12) as usize, rng.range(0, 12) as usize);
        let a = rng.rects(na, 54, 14);
        let b = rng.rects(nb, 54, 14);
        for op in [Op::And, Op::Or, Op::Not, Op::Xor] {
            let got: i64 = boolean(&a, &b, op).iter().map(|r| r.area()).sum();
            let want = raster_area(&a, &b, op, extent);
            assert_eq!(got, want, "trial {trial} op {op:?}: plane-sweep {got} != raster {want}\nA={a:?}\nB={b:?}");
        }
    }
}

#[test]
fn trace_contours_area_matches_boolean_union_on_random_inputs() {
    let mut rng = Lcg(0xD1B54A32D192ED03);
    for trial in 0..200 {
        let n = rng.range(3, 20) as usize;
        let rects = rng.rects(n, 80, 18);
        // Union as tiling rectangles, then the merged oriented rings of that union.
        let empty: Vec<Rect> = vec![];
        let tiles = boolean(&rects, &empty, Op::Or);
        let tile_area: i64 = tiles.iter().map(|r| r.area()).sum();
        let rings = trace_contours(&tiles);
        // Net signed 2x-area of the rings (outer CCW +, holes CW -) == 2x union area.
        let ring_area2: i64 = rings
            .iter()
            .map(|ring| {
                let n = ring.len() - 1;
                (0..n).map(|i| ring[i].0 as i64 * ring[i + 1].1 as i64 - ring[i + 1].0 as i64 * ring[i].1 as i64).sum::<i64>()
            })
            .sum();
        assert_eq!(ring_area2, 2 * tile_area, "trial {trial}: contour area {ring_area2} != 2x union {}", 2 * tile_area);
    }
}

/// Grid of ~10^5 disjoint rects: stresses the contour vertex split (2b).
fn grid(side: i32) -> Vec<Rect> {
    let mut v = Vec::with_capacity((side * side) as usize);
    for i in 0..side {
        for j in 0..side {
            let (x, y) = (i * 10, j * 10);
            v.push(Rect::new(x, y, x + 6, y + 6));
        }
    }
    v
}

/// ~10^5 rects with distinct left-x all overlapping in y: stresses the boolean
/// coverage sweep (2a) — every slab used to re-filter+resort all crossed edges.
fn stagger(n: i32) -> Vec<Rect> {
    (0..n).map(|i| Rect::new(2 * i, i, 2 * i + n, i + 50)).collect()
}

#[test]
fn boolean_scales_to_1e5_shapes() {
    let rects = stagger(100_000);
    let empty: Vec<Rect> = vec![];
    let t = Instant::now();
    let out = boolean(&rects, &empty, Op::Or);
    let elapsed = t.elapsed();
    assert!(!out.is_empty());
    // ~0.06s observed; O(N^2) needed minutes. 10s is a loose regression tripwire.
    assert!(elapsed.as_secs_f64() < 10.0, "boolean on 1e5 shapes took {elapsed:?} (O(N^2) regression?)");
}

#[test]
fn trace_contours_scales_to_1e5_shapes() {
    let rects = grid(316); // 99_856 rects
    let t = Instant::now();
    let rings = trace_contours(&rects);
    let elapsed = t.elapsed();
    assert_eq!(rings.len(), rects.len(), "disjoint grid: one ring per rect");
    // ~0.12s observed; O(N^2) needed tens of seconds. 10s is a loose tripwire.
    assert!(elapsed.as_secs_f64() < 10.0, "trace_contours on 1e5 shapes took {elapsed:?} (O(N^2) regression?)");
}
