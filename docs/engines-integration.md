# vyges-layout in the Vyges engine flow

`vyges-layout` is the **geometry substrate** — not an analysis engine, the kernel the
layout-side tools build on.

```text
                          ┌───────────────► vyges-lvs (Phase 2): GDS → devices + nets
   GDSII  ─► vyges-layout ┼───────────────► DRC (width/spacing), metal fill  [future]
   OASIS  ◄─(read/boolean/└───────────────► chip viewer: GDS parse for the web map
             flatten/convert)
```

Input and output are **GDSII or OASIS** (picked by file extension), so the kernel also
serves as a GDS↔OASIS converter for the rest of the flow.

## What it unblocks

- **`vyges-lvs` native extraction (Phase 2).** LVS's layout side is GDS → a transistor
  netlist: recognize devices from shapes on the device layers (boolean intersections of
  diff/poly/implant), and trace nets through metal/via layers. Both are boolean +
  connectivity on the geometry — exactly what this kernel provides. Today `vyges-lvs`
  consumes an externally-extracted SPICE netlist; with `vyges-layout` it extracts its own.
- **DRC / metal fill.** Width and spacing checks, and fill generation, are boolean +
  sizing + region queries on layers — the same kernel.
- **The chip viewer.** Its Phase-2 "view any GDSII" tool needs a GDS parser; this is the
  real, memory-safe Rust one (replacing the vendored JS parser).

## Where it sits

`vyges-layout` runs at/after layout, as a library + CLI. It does not replace KLayout for
interactive viewing or the golden DRC/LVS decks — it is the open kernel the Vyges layout
tools compose on. Pair with `vyges-lvs` (compare), and the sign-off engines
(`vyges-sta-si`, `vyges-power`/`vyges-em-ir`, `vyges-extract`).
