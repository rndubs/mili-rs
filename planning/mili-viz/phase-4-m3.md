# `mili-viz` Phase 4 M3 — primal result display (buildable scope)

> Scope doc for Phase 4 Milestone 3, continuing
> [`phase-4-m2.md`](phase-4-m2.md). M2 delivered the per-state
> triangulated hull through the frozen `ResultState.geometry`
> `GeometryRef`. M3 makes `show <svar>` carry a **scalar field** so a
> client can color the mesh. No proto change — the M1 contract is
> frozen; the scalar rides the same server-internal geometry blob, the
> `GeometryRef.layout` string already exists to version it.
>
> Read [`status.md`](status.md) first, then `phase-4-m2.md`. Reference
> behavior is read-only griz under `reference/griz/Src/`. Decisions
> continue the log (M1: 1–9; M2: 10–12; M3 starts at 13).

## Goal

`show <result> [component]` after a `load`:

- resolves the result to a `mili-rs` primal query at the current
  state,
- maps the queried values onto the mesh as a per-vertex scalar,
- delivers that scalar in the geometry blob and sets
  `ResultState.{min,max}` to the data range (griz autoscale),

so a Phase-5 client maps scalar → colormap → pixels. The server has no
colormap (that is a client render decision, README § "Why split");
M3's contract output is the **scalar field + its range**, not RGBA.

Out of scope (unchanged from `phase-4-m2.md`): selection/material
geometry effects (M4), derived results (M5), Flight-over-TCP (M6), the
client colormap/legend rendering (Phase 5).

## Decisions (continuing the log)

### Decision 13 — `show` resolves the leaf scalar svar (`component` if set, else `result`), finds its class via `classes_of_state_variable`, and an unresolvable result silently falls back to the M2 bare hull

`Command.Show` carries `result` + `component` but no class; griz
infers the class from the svar's subrecord bindings
(`reference/griz/Src/interpret.c` `parse_command` → `load_result`).
`mili-rs` already exposes the exact upstream resolution:
`Database::classes_of_state_variable(svar)`.

**Decision: the queried svar is `component` when non-empty, otherwise
`result` (griz's leaf-scalar `show sx` semantics — the proto's split
is for the vec-array/IP case, the leaf is what is queried). The class
is the first element class returned by
`classes_of_state_variable(svar)` that is also present in the M2
prepped topology; if the svar resolves to the `node` class it is a
nodal field. A result that resolves to no class, is an unknown svar,
or whose query errors falls back to the M2 bare hull (no scalar, the
`MVG1` layout) — `show` of an unknown name still draws the mesh and
never errors, matching griz (it warns and keeps the mesh shown).**
This keeps `show` total (the M2 "any/empty result draws the mesh"
invariant holds) and adds no class field to the frozen proto.

### Decision 14 — M3 colors by one scalar; the blob gains an optional per-vertex `scalar_f32` array (`MVG2` layout); element results are nodal-averaged, nodal results map directly

The M2 blob is server-internal (the proto carries only
`flight_ticket` + `layout` + counts); `layout` exists precisely to
version it (`phase-4-m1.md` § proto). M2 froze
`MVG1:verts_f32x3+idx_u32+trimat_u32`.

**Decision: when a scalar is present the blob appends a per-vertex
`f32` array and the layout becomes
`MVG2:verts_f32x3+idx_u32+trimat_u32+scalar_f32`; with no scalar it
stays exactly the M2 `MVG1` (so the M2 test and the bare-mesh path are
unchanged). Element results are mapped to vertices by **nodal
averaging** — each vertex's scalar is the mean of the incident
elements' values (griz's default smooth/interpolated shading,
`reference/griz/Src/draw.c`); nodal results map node→vertex directly.
A vertex touched by no resulted element is `f32::NAN` (the client
renders it as the mesh/edge, not a colored face). A multi-component
queried svar (e.g. a vector like `nodvel`) colors by component 0;
full component selection is past "primal result display" and the
proto already carries `component` for the leaf-svar case.**

**Trade-off recorded.** Per-element flat scalars (no averaging) would
be exact for piecewise-constant element fields, but the blob is
vertex-indexed (M2 Decision 11) and a per-triangle scalar array would
duplicate the M2 format decision for one milestone's benefit; griz's
default display is the nodal-averaged smooth field anyway. Per-element
flat shading is a Phase-5 client toggle over the same data, not an M3
contract change.

### Decision 15 — `ResultState.{min,max}` is the queried scalar's data range at the current state; the `legend` command stays a client-side display override

**Decision: `ResultState.min`/`max` are the min/max of the finite
queried scalar values at the current state (griz autoscale). The
existing `legend` `Command` (explicit limits) is a client-side display
clamp over this same range and adds nothing to M3's server scope —
M3's job is to report the true data range; clamping the colormap is
the renderer's (README § "Why split").**

## M3 acceptance gate

- [x] `show <element-scalar>` (e.g. `sand`/`brick`) after `load`
      yields `layout == "MVG2:..."`, a fetchable blob whose
      per-vertex scalar array has `num_vertices` entries, and
      `ResultState.{min,max}` bracketing the finite values.
- [x] `show <nodal-vector>` (e.g. `nodvel`) colors by component 0;
      the scalar array is finite for mesh nodes.
- [x] `show` with an empty/unknown result still draws the M2 bare
      hull (`MVG1`, no scalar) — no error, mesh unchanged.
- [x] Scalar tracks the state: the array differs between two states
      on a transient corpus.
- [x] All six M1 acceptance tests + the M2 test still pass unchanged
      (`MVG1` path untouched).
- [x] New test follows the CLAUDE.md skip-on-absent discipline.
      → `crates/mili-viz-server/tests/m3_primal.rs`
      `primal_result_colors_the_mesh`
- [x] `status.md` M3 box flipped with the gating test named;
      `README.md` open-questions table unaffected (no proto change).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 13 | `show` queries the leaf svar (`component`→`result`), class via `classes_of_state_variable`; unresolvable → bare hull | M3 resolution |
| 14 | One scalar; optional per-vertex `scalar_f32` (`MVG2`); element→nodal-averaged, nodal→direct; `NaN` for untouched; vector→comp 0 | M3 format |
| 15 | `ResultState.{min,max}` = data range (autoscale); `legend` is a client display clamp | M3 range |
