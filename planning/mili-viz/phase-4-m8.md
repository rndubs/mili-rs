# Phase 4 M8 — cut-plane operator (server-side clip)

> **Status: 🟡 PLANNED.** Second slice of the post-MVP server feature
> batch. Requires [`phase-4-m7.md`](phase-4-m7.md) (the `MVG3` layout
> is the carrier — the cut geometry rides the existing
> `ResultState.geometry` `GeometryRef`). Client UI in
> [`phase-5-m8.md`](phase-5-m8.md). Live status in
> [`status.md`](status.md). Decisions start at **75**.

## Why

`Cmd::Cutplane` (`mili_viz.proto:220-224`) has been in the frozen
proto since [`phase-4-m1.md`](phase-4-m1.md) Δ1 and the server's
dispatch arm at `crates/mili-viz-server/src/lib.rs:528` has been a
no-op stub (re-broadcasts the current `ResultState` unchanged).
griz's `cutpln` / `cutrpln` (`reference/griz/Src/contour.c`,
`reference/griz/Src/cutplane.c`) are the spec: a plane intersected
against the volumetric mesh yields a **closed clipped hull** =
{boundary faces of elements on the keep side} ∪ {polygonal cap
where the plane cuts through each element}. VisIt's "Clip" operator
is the same shape.

The compute belongs server-side (per maintainer direction —
"keep all of the rendering server side so that we can scale with
compute"): the server has the full element connectivity and the
worker pool; the client streams the resulting hull and renders it
through the unchanged Phase 5 pipeline.

## What lands

- `Cmd::Cutplane` arm in `crates/mili-viz-server/src/lib.rs`
  replaces the stub: parses the plane (origin + normal, absolute or
  data-relative — the existing `relative` field carries `cutpln` vs
  `cutrpln` semantics), runs the clip, and broadcasts a
  `DELTA_RESULT` whose `GeometryRef` points at the clipped hull
  blob (`MVG3`). The current `ResultState.{result, component, min,
  max}` is preserved — clipping is a **geometry** operation; the
  scalar / colormap path stays byte-stable.
- A new server module `crates/mili-viz-server/src/clip.rs` owns the
  per-superclass clip routine. The corpus is what `MeshTopology`
  already cached; the clip walks elements, classifies each by the
  signed distance of its corner nodes to the plane, and emits:
  - **All-keep** elements: unchanged outward boundary faces
    (same as today's hull pass).
  - **All-discard** elements: nothing.
  - **Straddling** elements: the kept portion as a marching-tets-
    equivalent table per superclass (Hex/Tet/Wedge/Pyramid each
    get one fixed dispatch table — 8/16/32/16 corner-sign cases
    respectively, mirroring `iso_surface.c`'s lookup-table shape;
    Tri/Quad surface elements degenerate to a 2D line segment and
    do not contribute to a 3D clipped hull).
- Cut-face caps are tessellated and emitted with `tri_material =
  u32::MAX - 1` (a reserved sentinel that the colormap pipeline
  treats as "neutral grey" — the cap is a synthetic surface, not a
  material face). Cap edges go in `MVG3.element_edges` so the
  wireframe pass shows the cap boundary cleanly.
- Cut state is session-level (`Session.cut: Option<CutPlane>`);
  every subsequent `show` / state step / material-toggle
  re-applies the clip on the next geometry emit. A second
  `cutpln` replaces; `cutpln` with the all-zero normal clears.

## Decisions

### Decision 75 — the cut produces **closed** geometry (kept-side faces ∪ cap), not just the kept boundary; the cap is one tessellated polygon per straddled element

griz's `cutpln` shows the cut face as part of the hull (a closed
silhouette); a "kept side only with a gaping hole" looks broken on
a real mesh. The cap is constructed per-element from the
plane-intersection polygon (3–6 vertices for a Hex straddler) and
fan-triangulated from its centroid. The cap polygon is **convex by
construction** (a convex element intersected with a plane is a
convex 2D polygon), so the fan triangulation is exact — no
constrained Delaunay needed.

**Trade-off recorded.** A "cut surface only" mode (no kept-side
boundary, just the cap) is what VisIt calls "Slice" — that is
[`phase-4-m9.md`](phase-4-m9.md), a separate verb, not a flag on
this one. Conflating them costs UI clarity (the wireframe spec
already separates "Cut" from "Slice"); the cleanest split is one
operator per semantics.

### Decision 76 — clip math runs **per-element with `rayon`**; no global mesh remesh, no spatial index built; the upper bound is `O(elements)` per cut, parallelizable

A clip plane touches every element at most once (sign-classify the
8 corners, dispatch a marching table, emit). No element-element
coupling, no spatial acceleration structure required — the trivial
parallel-for-each over the cached `MeshTopology.classes` saturates
a server-side `rayon` pool. Per griz `cutplane.c`, the LLNL
production corpora hit this regime cleanly (a few million elements
clipped in <1 s on a single node).

**Trade-off recorded.** A BVH-accelerated clip was rejected as
premature: it pays back only when a single workstation cuts repeatedly
against a static mesh, which is the **interactive-gizmo** workflow
([`phase-5-m8.md`](phase-5-m8.md)) — and even then, the
`MeshTopology` cache is already keyed per-state, so a re-cut on a
deforming state re-walks the cache anyway. If interactive-gizmo
profiling shows the per-element pass dominating, a per-state BVH
goes in as an additive optimization without touching the contract.

### Decision 77 — `cutpln` is a session-state verb that **composes** with `show` / state-step / `enable`/`disable`; the cut survives across results and states

When `cutpln` is set, every subsequent `DELTA_RESULT`-producing
command (`show`, `step`, `enable`/`disable`, `colormap`, `legend`,
`render`) re-applies the clip on the next geometry emit. The cap
is recomputed per state because the mesh deforms (cap polygons
move with the corner nodes). Clearing is the all-zero-normal
`cutpln` (per griz convention) or `cutpln` followed by
`cutpln off` — both shapes lower to the same `Session.cut = None`.

**Trade-off recorded.** Making the cut a one-shot ("clip the
current result, then forget the plane") was rejected: it forces the
client to re-issue the plane on every state step, which is a
guaranteed round-trip per frame during animation. Server-side
persistent state is the cheaper end-to-end shape (it is also how
`Cmd::Material` and the camera already work).

## Gating test

`crates/mili-viz-server/tests/m8_cutplane.rs::cutplane_operator`
— skip-on-absent against `bar71.pltA` (Hex corpus): asserts the
clipped hull has (a) fewer triangles than the unclipped hull,
(b) at least one triangle with `tri_material == u32::MAX - 1`
(the cap sentinel), (c) every cap triangle's three vertices lie
within `1e-5` of the plane equation, (d) every kept-side
triangle's three vertices satisfy `signed_distance >= -1e-5`
(plane keeps the positive half-space by convention; `relative`
toggles the sign).

## Trade-off recorded (milestone-level)

The compute is on the server (per the architectural direction);
the client just receives an `MVG3` blob through the unchanged
`fetch_geometry` seam. The visible cost is bandwidth on every cut
update (interactive drag → one blob per gesture-throttled frame);
the in-process transport is free, and the gRPC + Flight path
(landed M6) streams the blob without any new RPC. A future
"server-side gesture throttle" (debounce repeated `cutpln`
commands within a small wall-clock window) is the obvious
follow-up if interactive drag floods the bus.
