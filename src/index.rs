//! A spatial index over axis-aligned rectangles — the region-query primitive the
//! layout-side engines need to scale.
//!
//! DRC (width/spacing), LVS net tracing, and the viewer all ask the same question:
//! *"which shapes are near this one?"* Answering it by scanning all N shapes is
//! O(N) per query — O(N²) across a layer. A **uniform grid** buckets each rectangle
//! into the cells it covers, so a region query touches only the handful of cells the
//! region spans, not the whole layer.
//!
//! Grid (vs an R-tree) is deliberate for v0: layout shapes are numerous, small, and
//! roughly uniform, which is exactly where a grid wins — O(1) insert, cache-friendly,
//! no tree rebalancing. The index is **static** (build once over a shape set, then
//! query); incremental update is depth. It is a candidate for extraction into a
//! shared geometry crate once a second engine consumes it directly.
//!
//! ```
//! use vyges_layout::geom::Rect;
//! use vyges_layout::index::RegionIndex;
//! let rects = vec![Rect::new(0, 0, 10, 10), Rect::new(100, 100, 110, 110)];
//! let idx = RegionIndex::build(&rects);
//! assert_eq!(idx.overlaps(&Rect::new(5, 5, 50, 50)), vec![0]); // only the first
//! ```

use crate::geom::Rect;

/// A uniform-grid spatial index over a fixed set of rectangles (referenced by their
/// index in the slice passed to [`RegionIndex::build`]).
pub struct RegionIndex {
    ox: i32,
    oy: i32,
    cell: i64, // grid pitch (dbu); i64 to keep cell arithmetic overflow-free
    cols: i64,
    rows: i64,
    buckets: Vec<Vec<u32>>,
    rects: Vec<Rect>,
}

impl RegionIndex {
    /// Build an index over `rects`. The grid pitch is chosen from the shapes' average
    /// size so a typical shape spans ~1 cell; the grid is capped so pathological
    /// inputs cannot allocate an unbounded number of buckets.
    pub fn build(rects: &[Rect]) -> RegionIndex {
        if rects.is_empty() {
            return RegionIndex { ox: 0, oy: 0, cell: 1, cols: 1, rows: 1, buckets: vec![vec![]], rects: vec![] };
        }

        // extent + average shape size
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let (mut sw, mut sh) = (0i64, 0i64);
        for r in rects {
            x0 = x0.min(r.x0);
            y0 = y0.min(r.y0);
            x1 = x1.max(r.x1);
            y1 = y1.max(r.y1);
            sw += (r.x1 - r.x0) as i64;
            sh += (r.y1 - r.y0) as i64;
        }
        let n = rects.len() as i64;
        let avg = ((sw / n) + (sh / n)).max(1) / 2;
        let mut cell = avg.max(1);

        let ext_w = (x1 - x0) as i64;
        let ext_h = (y1 - y0) as i64;
        // cap the grid at ~4M cells: grow the pitch until cols*rows fits.
        const MAX_CELLS: i64 = 4_000_000;
        loop {
            let cols = ext_w / cell + 1;
            let rows = ext_h / cell + 1;
            if cols.saturating_mul(rows) <= MAX_CELLS {
                break;
            }
            cell *= 2;
        }
        let cols = ext_w / cell + 1;
        let rows = ext_h / cell + 1;

        // Bucket the shapes using local closures (before the struct exists, so there
        // is no aliasing between the cell-range walk and the mutable bucket push).
        let col = |x: i32| (((x - x0) as i64) / cell).clamp(0, cols - 1);
        let row = |y: i32| (((y - y0) as i64) / cell).clamp(0, rows - 1);
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); (cols * rows) as usize];
        for (i, r) in rects.iter().enumerate() {
            for cy in row(r.y0)..=row(r.y1) {
                for cx in col(r.x0)..=col(r.x1) {
                    buckets[(cy * cols + cx) as usize].push(i as u32);
                }
            }
        }
        RegionIndex { ox: x0, oy: y0, cell, cols, rows, buckets, rects: rects.to_vec() }
    }

    /// Candidate ids whose grid cells the `region` touches — a superset of the true
    /// hits (deduplicated). Cheap; use when a following exact test is applied anyway.
    pub fn query(&self, region: &Rect) -> Vec<u32> {
        let mut out = Vec::new();
        self.for_cells(region, |b| {
            for &id in &self.buckets[b] {
                out.push(id);
            }
        });
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Ids of indexed rectangles that share positive area with `region`.
    pub fn overlaps(&self, region: &Rect) -> Vec<u32> {
        let mut v: Vec<u32> =
            self.query(region).into_iter().filter(|&id| self.rects[id as usize].intersects(region)).collect();
        v.sort_unstable();
        v
    }

    /// True if any indexed rectangle overlaps `region`, optionally ignoring one id
    /// (e.g. a shape must not test against itself). Short-circuits.
    pub fn any_overlap(&self, region: &Rect, exclude: Option<u32>) -> bool {
        self.query(region).into_iter().any(|id| Some(id) != exclude && self.rects[id as usize].intersects(region))
    }

    /// Ids of rectangles within `dist` of `region` (a spacing halo) — overlaps of the
    /// region inflated by `dist`. `exclude` drops one id (typically the query shape).
    pub fn within(&self, region: &Rect, dist: i32, exclude: Option<u32>) -> Vec<u32> {
        let halo = region.inflate(dist);
        self.overlaps(&halo).into_iter().filter(|&id| Some(id) != exclude).collect()
    }

    /// Number of indexed rectangles.
    pub fn len(&self) -> usize {
        self.rects.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Visit each grid-cell index a rectangle's cell-range covers (clamped to grid).
    fn for_cells(&self, r: &Rect, mut f: impl FnMut(usize)) {
        let cx0 = self.col(r.x0);
        let cx1 = self.col(r.x1);
        let cy0 = self.row(r.y0);
        let cy1 = self.row(r.y1);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                f((cy * self.cols + cx) as usize);
            }
        }
    }

    fn col(&self, x: i32) -> i64 {
        (((x - self.ox) as i64) / self.cell).clamp(0, self.cols - 1)
    }
    fn row(&self, y: i32) -> i64 {
        (((y - self.oy) as i64) / self.cell).clamp(0, self.rows - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_index_answers_empty() {
        let idx = RegionIndex::build(&[]);
        assert!(idx.is_empty());
        assert!(idx.query(&Rect::new(0, 0, 10, 10)).is_empty());
        assert!(idx.overlaps(&Rect::new(0, 0, 10, 10)).is_empty());
        assert!(!idx.any_overlap(&Rect::new(0, 0, 10, 10), None));
    }

    #[test]
    fn finds_only_overlapping() {
        let rects = vec![Rect::new(0, 0, 10, 10), Rect::new(100, 100, 110, 110), Rect::new(5, 5, 15, 15)];
        let idx = RegionIndex::build(&rects);
        // region over the lower-left cluster hits 0 and 2, not the far one
        assert_eq!(idx.overlaps(&Rect::new(6, 6, 8, 8)), vec![0, 2]);
        // touching edge is not an overlap
        assert!(idx.overlaps(&Rect::new(10, 10, 20, 20)).iter().all(|&id| id != 0));
        assert_eq!(idx.overlaps(&Rect::new(105, 105, 106, 106)), vec![1]);
    }

    #[test]
    fn any_overlap_excludes_self() {
        let rects = vec![Rect::new(0, 0, 10, 10)];
        let idx = RegionIndex::build(&rects);
        assert!(idx.any_overlap(&rects[0], None));
        assert!(!idx.any_overlap(&rects[0], Some(0))); // only shape, excluded
    }

    #[test]
    fn within_is_a_spacing_halo() {
        let rects = vec![Rect::new(0, 0, 10, 10), Rect::new(15, 0, 25, 10)];
        let idx = RegionIndex::build(&rects);
        // gap between them is 5; shape 0 has no neighbour within 4 but does within 6
        assert!(idx.within(&rects[0], 4, Some(0)).is_empty());
        assert_eq!(idx.within(&rects[0], 6, Some(0)), vec![1]);
    }

    /// Deterministic grid of rectangles: the index must agree with brute force on
    /// many queries (the correctness oracle).
    #[test]
    fn matches_brute_force() {
        let mut rects = Vec::new();
        for gy in 0..40 {
            for gx in 0..40 {
                let x = gx * 25;
                let y = gy * 25;
                // varied sizes so some overlap across cells, some don't
                let w = 8 + (gx * 3 + gy) % 20;
                let h = 8 + (gy * 5 + gx) % 18;
                rects.push(Rect::new(x, y, x + w, y + h));
            }
        }
        let idx = RegionIndex::build(&rects);

        let queries = [
            Rect::new(0, 0, 30, 30),
            Rect::new(200, 200, 260, 205),
            Rect::new(-50, -50, 5, 5),
            Rect::new(500, 500, 900, 900),
            Rect::new(12, 37, 800, 62),
        ];
        for q in &queries {
            let mut brute: Vec<u32> =
                (0..rects.len() as u32).filter(|&i| rects[i as usize].intersects(q)).collect();
            brute.sort_unstable();
            assert_eq!(idx.overlaps(q), brute, "mismatch on {q:?}");
        }
    }
}
