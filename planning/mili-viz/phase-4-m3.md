# Phase 4 M3 — landed (primal result display)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `show <result> [component]` after `load` resolves the leaf scalar
  svar (`component` if non-empty, else `result`), finds its element
  class via `Database::classes_of_state_variable`, queries the primal
  at the current state, and maps the values onto the mesh as a
  per-vertex scalar.
- Element results are nodal-averaged onto vertices (griz's default
  smooth shading); nodal results map node→vertex directly; vertices
  with no resulted incident element are `f32::NAN`; multi-component
  vector svars color by component 0.
- Blob format extended with an optional per-vertex `scalar_f32`
  array; layout becomes `MVG2:verts_f32x3+idx_u32+trimat_u32+scalar_f32`
  when a scalar is present, falls back to the M2 `MVG1` when not (so
  unknown/empty `show` still draws the bare hull — `show` never
  errors).
- `ResultState.{min,max}` carries the griz autoscale (the finite-data
  range at the current state); the `legend` command stays a
  client-side display clamp over this range.

## Gating test

`crates/mili-viz-server/tests/m3_primal.rs::primal_result_colors_the_mesh`
— asserts `MVG2` layout, per-vertex scalar of `num_vertices` length,
range bracketing, vector-svar component-0 selection, state tracking,
and bare-hull fallback for unknown results.

## Decisions

- Decisions 13–15 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR.
