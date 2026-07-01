//! vyges-layout — a layout geometry kernel.
//!
//! The memory-safe Rust substrate the other layout-side tools ride on: read and
//! write **GDSII**, do **polygon boolean** operations (the workhorse of DRC, fill,
//! and device extraction), and **flatten** a cell hierarchy. It is the dependency
//! that unblocks `vyges-lvs` native layout extraction (GDS → devices) and replaces
//! the vendored JS GDS parser in the Vyges chip viewer.
//!
//! Boundaries (per the Vyges flow architecture): files in, files/reports out, pure
//! std, unit-tested offline — no subprocess. Peers are libraries (KLayout-db, gdstk),
//! not products; this is the clean-room Rust one.
//!
//! v0 scope: a GDSII reader/writer (round-trip), an **OASIS reader/writer** (the
//! compact modern interchange format — RECTANGLE / POLYGON / PLACEMENT subset),
//! per-layer stats (`info`), hierarchy flatten (SREF; AREF arrays), and **Manhattan
//! boolean** (AND/OR/NOT/XOR) on axis-aligned rectangles via a scanline, and a
//! **spatial index** (`index`) for region/overlap/spacing-halo queries. Depth
//! reserved: rectilinear-polygon decomposition + general-angle clipping (Vatti),
//! full third-party OASIS ingest, DRC width/spacing *rules* on top of the region
//! index, and net tracing for extraction.

pub mod gds;
pub mod oasis;
// `geom` and `index` live in the shared `vyges-geom` crate; re-export them so the
// layout-side engines keep using `vyges_layout::geom` / `::index` unchanged.
pub use vyges_geom::{geom, index};
pub mod boolean;
pub mod flatten;
pub mod engine;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COPYRIGHT: &str = "© 2026 Vyges. All Rights Reserved.  https://vyges.com";
