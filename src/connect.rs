//! Shared connectivity / net tracing — layer-tagged geometry into connected nets.
//!
//! One union-find kernel used by every layout engine that needs "which shapes form
//! one electrical net": device extraction (LVS) and RC parasitic extraction both
//! ride this. The caller prepares the connective geometry per layer (so a device
//! flow can pass diffusion-minus-poly for correct source/drain separation, and an
//! RC flow can pass routing rects) and describes how cut/via layers join:
//!
//! - [`Cut::Enclosed`] — the cut joins layers `a` and `b` only where it is *enclosed*
//!   by a shape on each (DRC-clean contact/via — device extraction).
//! - [`Cut::Overlap`] — the cut joins any shapes it *overlaps* (looser — RC tracing).
//!
//! [`trace`] returns per-layer **prims** (a prim is one same-layer connected component,
//! true-tiled), a **net id** per prim, an optional **name** per net (from a TEXT label),
//! and a per-net **cut count**. Consumers add their own interpretation on top (roles +
//! device detection, or per-net segments for RC).

use std::collections::{BTreeMap, HashMap};

use crate::boolean::{boolean_poly, Op};
use crate::geom::{self, Rect};

/// GDS layer + datatype.
pub type Ld = (i16, i16);
/// A rectilinear polygon (closed ring), in DB units.
pub type Poly = Vec<(i32, i32)>;

/// How a cut/via layer's geometry joins the connective layers.
pub enum Cut {
    /// Join `a` and `b` where a cut shape is enclosed by a shape on each (DRC-clean).
    Enclosed { cut: Vec<Poly>, a: Ld, b: Ld },
    /// Join any connective shapes a cut shape overlaps (looser).
    Overlap { cut: Vec<Poly> },
}

/// One same-layer connected component (its true-tiled geometry).
pub struct Prim {
    pub layer: Ld,
    pub rects: Vec<Rect>,
}

/// Result of tracing: prims, the net each belongs to, per-net names + cut counts.
pub struct Traced {
    pub prims: Vec<Prim>,
    pub net_id: Vec<usize>,          // net index per prim
    pub names: Vec<Option<String>>,  // name per net (None = anonymous)
    pub cut_count: Vec<usize>,       // cut/via shapes per net
    pub nnets: usize,
}

impl Traced {
    /// Net name, falling back to `n<id>` for an unnamed net.
    pub fn net_name(&self, nid: usize) -> String {
        self.names[nid].clone().unwrap_or_else(|| format!("n{nid}"))
    }
}

// --------------------------------------------------------------------------
// geometry helpers (shared kernel)
// --------------------------------------------------------------------------

fn bbox_of(p: &[(i32, i32)]) -> Rect {
    geom::bbox(p).unwrap_or(Rect { x0: 0, y0: 0, x1: 0, y1: 0 })
}
fn overlap(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}
pub(crate) fn union_bbox(rects: &[Rect]) -> Rect {
    let mut r = rects[0];
    for x in &rects[1..] {
        r.x0 = r.x0.min(x.x0);
        r.y0 = r.y0.min(x.y0);
        r.x1 = r.x1.max(x.x1);
        r.y1 = r.y1.max(x.y1);
    }
    r
}
fn contains(o: &Rect, c: &Rect) -> bool {
    o.x0 <= c.x0 && c.x1 <= o.x1 && o.y0 <= c.y0 && c.y1 <= o.y1
}
/// Is `inner` fully covered by `outer` (a DRC-clean contact sits inside its metal)?
fn enclosed(inner: &[Rect], outer: &[Rect]) -> bool {
    if inner.is_empty() || outer.is_empty() || !overlap(&union_bbox(inner), &union_bbox(outer)) {
        return false;
    }
    if inner.iter().all(|c| outer.iter().any(|o| contains(o, c))) {
        return true;
    }
    let ip: Vec<_> = inner.iter().map(|r| r.as_boundary()).collect();
    let op: Vec<_> = outer.iter().map(|r| r.as_boundary()).collect();
    boolean_poly(&ip, &op, Op::Not).is_empty()
}
/// Is `poly` a single axis-aligned rectangle (the common case)?
fn is_rect(poly: &[(i32, i32)]) -> bool {
    let p = if poly.len() >= 2 && poly.first() == poly.last() { &poly[..poly.len() - 1] } else { poly };
    if p.len() != 4 {
        return false;
    }
    let bb = bbox_of(poly);
    p.iter().all(|&(x, y)| (x == bb.x0 || x == bb.x1) && (y == bb.y0 || y == bb.y1))
}
/// Rect-tiling of a rectilinear polygon — preserves true geometry (vs a bbox).
pub fn tile(poly: &[(i32, i32)]) -> Vec<Rect> {
    if is_rect(poly) {
        return vec![bbox_of(poly)];
    }
    boolean_poly(&[poly.to_vec()], &[], Op::Or)
}
fn pt_in(rects: &[Rect], x: i32, y: i32) -> bool {
    rects.iter().any(|r| r.x0 <= x && x <= r.x1 && r.y0 <= y && y <= r.y1)
}

pub(crate) struct Uf {
    p: Vec<usize>,
}
impl Uf {
    pub(crate) fn new(n: usize) -> Uf {
        Uf { p: (0..n).collect() }
    }
    pub(crate) fn find(&mut self, x: usize) -> usize {
        if self.p[x] != x {
            let r = self.find(self.p[x]);
            self.p[x] = r;
        }
        self.p[x]
    }
    pub(crate) fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.p[a] = b;
        }
    }
}

/// Uniform-grid spatial index over axis-aligned boxes — overlap queries ~O(1).
pub(crate) struct Grid {
    cell: i64,
    minx: i64,
    miny: i64,
    buckets: HashMap<(i32, i32), Vec<usize>>,
    big: Vec<usize>,
}
const BIG_CELLS: i64 = 256;
impl Grid {
    pub(crate) fn build(boxes: &[Rect]) -> Grid {
        let n = boxes.len().max(1);
        let (mut minx, mut miny, mut maxx, mut maxy) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        let (mut wsum, mut hsum) = (0i64, 0i64);
        for r in boxes {
            minx = minx.min(r.x0 as i64);
            miny = miny.min(r.y0 as i64);
            maxx = maxx.max(r.x1 as i64);
            maxy = maxy.max(r.y1 as i64);
            wsum += (r.x1 - r.x0) as i64 + 1;
            hsum += (r.y1 - r.y0) as i64 + 1;
        }
        if boxes.is_empty() {
            (minx, miny, maxx, maxy) = (0, 0, 0, 0);
        }
        let avg = ((wsum + hsum) / (2 * n as i64)).max(1);
        let span = (maxx - minx).max(maxy - miny).max(1);
        let cell = avg.max(span / 512).max(1);
        let mut g = Grid { cell, minx, miny, buckets: HashMap::new(), big: Vec::new() };
        for (i, r) in boxes.iter().enumerate() {
            let (cx0, cy0, cx1, cy1) = g.range(r);
            let ncells = (cx1 - cx0 + 1) as i64 * (cy1 - cy0 + 1) as i64;
            if ncells > BIG_CELLS {
                g.big.push(i);
            } else {
                for cx in cx0..=cx1 {
                    for cy in cy0..=cy1 {
                        g.buckets.entry((cx, cy)).or_default().push(i);
                    }
                }
            }
        }
        g
    }
    fn range(&self, r: &Rect) -> (i32, i32, i32, i32) {
        let c = self.cell;
        (
            ((r.x0 as i64 - self.minx) / c) as i32,
            ((r.y0 as i64 - self.miny) / c) as i32,
            ((r.x1 as i64 - self.minx) / c) as i32,
            ((r.y1 as i64 - self.miny) / c) as i32,
        )
    }
    pub(crate) fn query(&self, r: &Rect, out: &mut Vec<usize>) {
        out.clear();
        let (cx0, cy0, cx1, cy1) = self.range(r);
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                if let Some(v) = self.buckets.get(&(cx, cy)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.extend_from_slice(&self.big);
        out.sort_unstable();
        out.dedup();
    }
}

/// Connected components of a rect set by true overlap, spatially indexed (~O(n)).
pub fn components(rects: &[Rect]) -> Vec<Vec<Rect>> {
    let n = rects.len();
    let mut uf = Uf::new(n);
    let g = Grid::build(rects);
    for idxs in g.buckets.values() {
        for a in 0..idxs.len() {
            for b in a + 1..idxs.len() {
                let (i, j) = (idxs[a], idxs[b]);
                if uf.find(i) != uf.find(j) && overlap(&rects[i], &rects[j]) {
                    uf.union(i, j);
                }
            }
        }
    }
    for &i in &g.big {
        for j in 0..n {
            if i != j && uf.find(i) != uf.find(j) && overlap(&rects[i], &rects[j]) {
                uf.union(i, j);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<Rect>> = HashMap::new();
    for i in 0..n {
        groups.entry(uf.find(i)).or_default().push(rects[i]);
    }
    groups.into_values().collect()
}

/// Trace connected nets.
///
/// `layers` is the connective geometry per layer (already the caller's choice of
/// shapes — e.g. diffusion-minus-poly for a device flow). `cuts` join those layers.
/// `labels` are `(name, layer, x, y)` points naming the net they land on; the
/// label's layer *number* is preferred when choosing which prim (net) it names, so
/// a met1 label (`68/5`) attaches to the met1 net (`68/20`), not a shape beneath it.
pub fn trace(layers: &[(Ld, Vec<Poly>)], cuts: &[Cut], labels: &[(String, i16, i32, i32)]) -> Traced {
    // 1. prims = same-layer connected components (true-tiled).
    let mut prims: Vec<Prim> = Vec::new();
    for (ld, polys) in layers {
        let mut tiles = Vec::new();
        for p in polys {
            tiles.extend(tile(p));
        }
        for comp in components(&tiles) {
            prims.push(Prim { layer: *ld, rects: comp });
        }
    }

    // 2. union prims through cuts.
    let n = prims.len();
    let mut uf = Uf::new(n);
    let prim_bb: Vec<Rect> = prims.iter().map(|p| union_bbox(&p.rects)).collect();
    let pgrid = Grid::build(&prim_bb);
    let mut cand: Vec<usize> = Vec::new();
    let mut cut_shapes: Vec<Vec<Rect>> = Vec::new(); // each cut shape's rects, for net attribution
    let mut cut_owner: Vec<Option<usize>> = Vec::new(); // a prim it touches (for net cut-count)
    for cut in cuts {
        match cut {
            Cut::Enclosed { cut, a, b } => {
                for cp in cut {
                    let ct = tile(cp);
                    pgrid.query(&union_bbox(&ct), &mut cand);
                    let pa = cand.iter().copied().find(|&pi| prims[pi].layer == *a && enclosed(&ct, &prims[pi].rects));
                    let pb = cand.iter().copied().find(|&pi| prims[pi].layer == *b && enclosed(&ct, &prims[pi].rects));
                    if let (Some(i), Some(j)) = (pa, pb) {
                        uf.union(i, j);
                    }
                    cut_owner.push(pa.or(pb));
                    cut_shapes.push(ct);
                }
            }
            Cut::Overlap { cut } => {
                for cp in cut {
                    let ct = tile(cp);
                    pgrid.query(&union_bbox(&ct), &mut cand);
                    let hits: Vec<usize> = cand
                        .iter()
                        .copied()
                        .filter(|&pi| prims[pi].rects.iter().any(|r| ct.iter().any(|c| overlap(r, c))))
                        .collect();
                    for w in hits.windows(2) {
                        uf.union(w[0], w[1]);
                    }
                    cut_owner.push(hits.first().copied());
                    cut_shapes.push(ct);
                }
            }
        }
    }
    let _ = cut_shapes;

    // 3. canonical net ids.
    let mut canon: BTreeMap<usize, usize> = BTreeMap::new();
    let net_id: Vec<usize> = (0..n)
        .map(|i| {
            let r = uf.find(i);
            let k = canon.len();
            *canon.entry(r).or_insert(k)
        })
        .collect();
    let nnets = canon.len();

    // 4. per-net cut count (a cut belongs to the net of a prim it joins).
    let mut cut_count = vec![0usize; nnets];
    for owner in &cut_owner {
        if let Some(pi) = owner {
            cut_count[net_id[*pi]] += 1;
        }
    }

    // 5. names from labels — prefer a prim on the label's own layer number, else any.
    let mut names: Vec<Option<String>> = vec![None; nnets];
    for (string, layer, x, y) in labels {
        let pi = prims
            .iter()
            .position(|p| p.layer.0 == *layer && pt_in(&p.rects, *x, *y))
            .or_else(|| prims.iter().position(|p| pt_in(&p.rects, *x, *y)));
        if let Some(pi) = pi {
            names[net_id[pi]] = Some(string.clone());
        }
    }

    Traced { prims, net_id, names, cut_count, nnets }
}
