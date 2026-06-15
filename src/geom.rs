//! Geometry primitives — axis-aligned rectangles, polygon area, bounding box.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32, // x1 > x0
    pub y1: i32, // y1 > y0
}

impl Rect {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn area(&self) -> i64 {
        (self.x1 - self.x0) as i64 * (self.y1 - self.y0) as i64
    }
    pub fn as_boundary(&self) -> Vec<(i32, i32)> {
        vec![
            (self.x0, self.y0),
            (self.x1, self.y0),
            (self.x1, self.y1),
            (self.x0, self.y1),
            (self.x0, self.y0),
        ]
    }
    /// `Some(Rect)` iff the polygon is an axis-aligned rectangle (2 distinct x, 2
    /// distinct y); else `None` (caller decides whether to bbox-approximate).
    pub fn from_boundary(pts: &[(i32, i32)]) -> Option<Rect> {
        let n = if pts.len() >= 2 && pts.first() == pts.last() { pts.len() - 1 } else { pts.len() };
        if n != 4 {
            return None;
        }
        let xs: std::collections::BTreeSet<i32> = pts[..n].iter().map(|p| p.0).collect();
        let ys: std::collections::BTreeSet<i32> = pts[..n].iter().map(|p| p.1).collect();
        if xs.len() == 2 && ys.len() == 2 {
            let xv: Vec<i32> = xs.into_iter().collect();
            let yv: Vec<i32> = ys.into_iter().collect();
            Some(Rect { x0: xv[0], y0: yv[0], x1: xv[1], y1: yv[1] })
        } else {
            None
        }
    }
}

/// Bounding box of a point set.
pub fn bbox(pts: &[(i32, i32)]) -> Option<Rect> {
    let first = pts.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for &(x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Some(Rect { x0, y0, x1, y1 })
}

/// Signed polygon area via the shoelace formula (absolute value).
pub fn poly_area(pts: &[(i32, i32)]) -> f64 {
    let n = if pts.len() >= 2 && pts.first() == pts.last() { pts.len() - 1 } else { pts.len() };
    if n < 3 {
        return 0.0;
    }
    let mut a: i64 = 0;
    for i in 0..n {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % n];
        a += x1 as i64 * y2 as i64 - x2 as i64 * y1 as i64;
    }
    (a.abs() as f64) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rectangle() {
        let r = Rect::from_boundary(&[(0, 0), (10, 0), (10, 5), (0, 5), (0, 0)]).unwrap();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (0, 0, 10, 5));
        assert_eq!(r.area(), 50);
        // an L-shape is not a rectangle
        assert!(Rect::from_boundary(&[(0, 0), (10, 0), (10, 5), (5, 5), (5, 10), (0, 10), (0, 0)]).is_none());
    }

    #[test]
    fn area_and_bbox() {
        let l = [(0, 0), (10, 0), (10, 5), (5, 5), (5, 10), (0, 10), (0, 0)];
        assert_eq!(poly_area(&l), 75.0); // 10x5 + 5x5
        let b = bbox(&l).unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0, 0, 10, 10));
    }
}
