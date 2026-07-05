# vyges-layout — kernel notes & scope vs KLayout-db / gdstk

`vyges-layout` is a layout-geometry **library** (with a CLI), the same role as KLayout's
database or gdstk — not an end-user product. The difference is **clean-room, memory-safe
Rust, std-only**, so the Vyges layout tools share one auditable base.

## What v0 does

| Capability | vyges-layout v0 | KLayout-db / gdstk |
| --- | --- | --- |
| GDSII read/write (round-trip) | ✅ BOUNDARY/PATH/SREF/AREF/BOX | ✅ full |
| OASIS write | ✅ RECTANGLE/POLYGON/PLACEMENT subset (GDS↔OASIS convert) | ✅ full |
| OASIS read (third-party) | ✅ full record set: RECTANGLE/POLYGON(+manhattan closure)/PATH/TRAPEZOID/CTRAPEZOID/CIRCLE/TEXT/PLACEMENT + repetitions + properties/name-tables + **CBLOCK (DEFLATE via miniz_oxide)**; validated on real sky130 (gdstk, compressed+raw) == GDS. Gap: rare CTRAPEZOID 20–23 (error, not guessed) | ✅ full |
| Per-layer stats (`info`) | ✅ | ✅ |
| Boolean AND/OR/NOT/XOR | ✅ Manhattan rectilinear polygons (scanline) | ✅ general polygons (Vatti/edge) |
| Hierarchy flatten | ✅ SREF/AREF, composed transforms | ✅ |
| Spatial index (region / overlap / spacing-halo queries) | ✅ uniform-grid `RegionIndex` | ✅ (R-tree) |
| Sizing + DRC width·spacing *rules* | ❌ (depth — on top of the index) | ✅ |
| Net tracing / device extraction | ❌ Phase 2 (the `vyges-lvs` seam) | ◐ (KLayout LVS) |

## Boolean: the Manhattan scanline (v0)

The boolean runs a **vertical scanline**: sweeping x left→right, the y-coverage of A and
of B is maintained **incrementally** — each edge is folded into a running interval set as
its x is crossed, rather than recomputed from all edges per slab — and between consecutive
edges the op is applied on those intervals and emitted as rectangles, then merged
horizontally. That makes it O((N+K) log N), so it holds at full-chip shape counts (10⁵+
per layer) rather than the O(N²) of a per-slab recompute. The companion boundary tracer
(`trace_contours`, tiles → merged oriented polygons) indexes vertices per supporting line
for the same reason. Integer coordinates make both exact; a non-rectilinear boundary is
bbox-approximated and **counted** in the report (no silent caps).

## Depth pass

1. **Rectilinear-polygon decomposition** → run the same scanline on L-shapes etc.
2. **General-angle clipping** (Vatti / Greiner-Hormann) for non-Manhattan geometry.
3. **DRC width/spacing rules** on top of the `RegionIndex` region/overlap/halo
   queries (the spatial-index primitive itself has landed); plus rectangle sizing
   (grow/shrink). `RegionIndex` is a candidate for extraction into a shared geometry
   crate once a second engine consumes it directly.
4. **Net tracing + device recognition** → the `vyges-lvs` Phase-2 extraction seam.
5. **Full third-party OASIS ingest** (TRAPEZOID/CTRAPEZOID/PATH/CBLOCK/properties, modal
   compaction, matrix repetitions) on top of the v0 RECTANGLE/POLYGON/PLACEMENT subset;
   per-layer render hooks for the chip viewer.

Honest bound: v0 is a real GDSII **and OASIS** kernel with exact Manhattan boolean and
flatten — a solid base for the layout tools — with general geometry, full OASIS ingest,
and extraction on the path above.
