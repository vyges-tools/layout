# vyges-layout — kernel notes & scope vs KLayout-db / gdstk

`vyges-layout` is a layout-geometry **library** (with a CLI), the same role as KLayout's
database or gdstk — not an end-user product. The difference is **clean-room, memory-safe
Rust, std-only**, so the Vyges layout tools share one auditable base.

## What v0 does

| Capability | vyges-layout v0 | KLayout-db / gdstk |
| --- | --- | --- |
| GDSII read/write (round-trip) | ✅ BOUNDARY/PATH/SREF/AREF/BOX | ✅ full |
| OASIS read/write | ✅ RECTANGLE/POLYGON/PLACEMENT subset (GDS↔OASIS convert) | ✅ full |
| Per-layer stats (`info`) | ✅ | ✅ |
| Boolean AND/OR/NOT/XOR | ✅ Manhattan rectilinear polygons (scanline) | ✅ general polygons (Vatti/edge) |
| Hierarchy flatten | ✅ SREF/AREF, composed transforms | ✅ |
| Sizing / region (DRC width·spacing) | ❌ (depth) | ✅ |
| Net tracing / device extraction | ❌ Phase 2 (the `vyges-lvs` seam) | ◐ (KLayout LVS) |

## Boolean: the Manhattan scanline (v0)

The boolean runs a **vertical scanline**: for each x-slab between consecutive rectangle
edges, the y-coverage of A and of B is an interval set; the op is applied on those
intervals and emitted as rectangles, then merged horizontally across slabs. Integer
coordinates make it exact. v0 inputs are axis-aligned rectangles; a non-rectangle
boundary is bbox-approximated and **counted** in the report (no silent caps).

## Depth pass

1. **Rectilinear-polygon decomposition** → run the same scanline on L-shapes etc.
2. **General-angle clipping** (Vatti / Greiner-Hormann) for non-Manhattan geometry.
3. **Sizing + region queries** (grow/shrink, width/spacing) → DRC primitives.
4. **Net tracing + device recognition** → the `vyges-lvs` Phase-2 extraction seam.
5. **Full third-party OASIS ingest** (TRAPEZOID/CTRAPEZOID/PATH/CBLOCK/properties, modal
   compaction, matrix repetitions) on top of the v0 RECTANGLE/POLYGON/PLACEMENT subset;
   per-layer render hooks for the chip viewer.

Honest bound: v0 is a real GDSII **and OASIS** kernel with exact Manhattan boolean and
flatten — a solid base for the layout tools — with general geometry, full OASIS ingest,
and extraction on the path above.
