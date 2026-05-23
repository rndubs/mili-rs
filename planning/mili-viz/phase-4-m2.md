# Phase 4 M2 — landed (load + state navigation + real geometry)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `mili-viz-server` depends on `mili-rs` (no `pyo3`/`parity`). `load
  <root>` opens a real `Database` and broadcasts a `LoadedState` with
  the real `num_states`, `state_times`, and element `class_names`.
- `state`/`next`/`prev`/`first`/`last` clamp the cursor to
  `[1, num_states]` (griz-faithful: an over-range `state` clamps, it
  does not error) when a database is loaded; unchanged otherwise.
- `show` extracts the per-state triangulated hull and delivers it
  through `ResultState.geometry = Some(GeometryRef)` with a real
  server-assigned `flight_ticket` (`geom:{seq}`), per-superclass
  corner triangulation (Hex→12, Wedge→8, Pyramid→6, Tet/Tet10→4,
  Quad→2, Tri→1), and per-state node positions from the primal
  `nodpos` query (falls back to reference `node_coords` if absent).
- Bulk geometry resolves through an in-process geometry store
  (`VizService::fetch_geometry`) keyed by the frozen
  `flight_ticket` — the real Arrow Flight `DoGet` wire is M6.
- Self-describing little-endian blob format frozen: layout
  `MVG1:verts_f32x3+idx_u32+trimat_u32` (magic `MVG1`, dims, n_verts,
  n_idx, verts, indices, per-triangle material).
- One `StateDelta` per `Execute` invariant preserved: `load` does
  not auto-`show`.

## Gating test

`crates/mili-viz-server/tests/m2_geometry.rs::load_state_nav_and_real_geometry`
— asserts real `LoadedState`, state-cursor clamping, a fetchable blob
that decodes per the MVG1 layout, and per-state variation on a
deforming corpus.

## Decisions

- Decisions 10–12 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR.
