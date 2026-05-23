# Phase 5 M7 — render modes consuming `MVG3` (translucent / X-ray / element-edges)

> **Status: 🟡 PLANNED.** Client side of the post-MVP volumetric
> feature batch. Requires the server-side
> [`phase-4-m7.md`](phase-4-m7.md) (the `MVG3` layout). Sibling to
> [`phase-5-m8.md`](phase-5-m8.md) (cut-plane UI) and
> [`phase-5-m9.md`](phase-5-m9.md) (slice UI). Live status in
> [`status.md`](status.md). Decisions start at **81**.

## Why

Three rendering modes are blocked behind the M2 `MVG1`/`MVG2`
surface-only contract:

1. **Faithful element wireframe.** Today's `Edges`/`Wireframe` mode
   draws triangle-edge derivatives (`Mesh::edge_indices`) and so
   surfaces the per-face triangulation diagonals as wireframe
   lines — the hex VB-005 artifact. With `MVG3.element_edges` the
   client draws what the server explicitly enumerated.
2. **Translucent whole-mesh.** Drawing the existing hull with
   `alpha < 1` shows only the silhouette of the outer surface — no
   internal cell structure. With `MVG3` interior triangles
   (Decision 74) the translucent pass renders cell-cell interfaces
   too, so the user sees the volumetric interior.
3. **X-ray / "see-through-but-edges-visible".** A two-pass mode
   that draws the translucent fill plus the full element-edge
   set — the high-information mode that griz `wireframe edges`
   approximates but cannot do faithfully because griz already
   triangulated.

All three are client-only when the server speaks `MVG3` —
**zero proto change**, decoder extension only, plus three new
`RenderMode` arms.

## What lands

- `crates/mili-viz-client/src/mesh.rs::decode_mvg` learns the
  `MVG3` magic and the four flag-bit sections; populates the new
  `Mesh::element_edges: Option<Vec<u32>>` and `Mesh::tri_flags:
  Option<Vec<u32>>`.
- `RenderMode` (`crates/mili-viz-client/src/shell.rs`) gains three
  new variants:
  - `Translucent` — fill pass with `alpha = 0.35` (a viz-local
    default tweakable through Preferences), drawn through the
    existing depth-tested pipeline but with the **order-independent**
    write-mode: depth test on, depth write off, blend
    `SrcAlpha`/`OneMinusSrcAlpha`. No sorting; on the corpora of
    interest (a few hundred thousand triangles) the rough silhouette
    is acceptable. A future sort/OIT polish is non-blocking.
  - `Xray` — `Translucent` fill **plus** the element-edge LineList
    pass (`MVG3.element_edges` if present, falling back to
    `Mesh::edge_indices`).
  - `Interior` (default off; toggled separately) — instructs the
    next geometry request to ask the server for the
    interior-triangle bit (Decision 74's sentinel — the client
    lowers a `Cmd::Material(MaterialVisibility{ material:
    Some(u32::MAX), enable: on, .. })`). `Interior` composes with
    any of `Shaded`/`Translucent`/`Xray` and adds the interior
    faces (drawn with the cap sentinel's neutral colour when
    `tri_flags & 1 != 0` and no scalar is mapped).
- The existing `Edges` and `Wireframe` modes (status.md item 23 /
  VB-003 / VB-004) **prefer** `Mesh::element_edges` when present;
  fall back to the on-the-fly extractor when not. This is the
  VB-005 fix on the client side — server side is
  [`phase-4-m7.md`](phase-4-m7.md) Decision 73.

## Decisions

### Decision 81 — `Translucent` is **un-sorted** OIT-equivalent (depth-test on, depth-write off, no triangle sort); a sorted/OIT polish is non-blocking

Pixar-style OIT or per-frame depth-sorted triangles cost a full
GPU pass plus a sort over the index buffer; the existing
`bar71.pltA`-class corpora render rough-translucent acceptably with
plain `SrcAlpha` over a depth-tested but non-writing pipeline. The
artifact is "occlusion order is not exact on overlapping
translucent triangles" — visible as minor shading inconsistency,
not as a structural error. The wireframe spec's
"Translucent/X-ray" entry does not require OIT correctness; it
requires "see internal structure", which the un-sorted pass
delivers.

**Trade-off recorded.** Weighted-blend OIT (one extra pass, one
extra render target, `wgpu` 29 supports natively) is the natural
upgrade path; it can land as a non-default opt-in (`Preferences
→ High-quality transparency`) without re-opening this milestone.

### Decision 82 — `Wireframe`/`Edges` modes prefer `MVG3.element_edges`, fall back to triangle-edge extraction; the fallback path is **byte-stable** with the M4 + MVP-polish gates

Today's `Edges` pipeline reads `Mesh::edge_indices()` (computed
from the triangle list). The new code path is one branch:

```rust
let edges = mesh.element_edges
    .as_deref()
    .unwrap_or(&mesh.edge_indices());  // legacy: derive from tris
renderer.draw_lines(edges);
```

For an `MVG2` blob (no `element_edges` section) the branch falls
to the fallback verbatim — every M4 / VB-003 / VB-004 / MVP-polish
composite gate stays byte-stable (VB-001).

**Discharges** [`bug-tracker.md`](bug-tracker.md) VB-005 on the
client side once a server speaking `MVG3` is connected; the
fallback path preserves the (broken-but-known) old behavior for
older servers.

### Decision 83 — the `Interior` toggle is a **server round-trip** (re-emits the geometry blob with interior triangles included); the client does not synthesize interior faces locally

The interior triangle list is the server's responsibility (only the
server has cell-cell adjacency, only the server can dedupe shared
faces). Toggling `Interior` lowers to a
`Cmd::Material(MaterialVisibility{ material: Some(u32::MAX),
enable: bool, .. })` and waits for the next `DELTA_RESULT`'s
re-emitted blob. The latency cost is one round trip per toggle —
acceptable for a "show me the inside" verb that is fundamentally a
geometry-shape change, not a per-frame state.

**Trade-off recorded.** A purely client-side "render every
back-face as translucent" trick (cull off, `alpha < 1`) was
considered — it gives a visual hint at internal structure without
a round trip — but it cannot show the **internal element
interfaces** (cell-cell shared faces between adjacent solids) since
those faces are absent from the boundary hull. Server round-trip
is the only path to a faithful answer.

## Gating test

`crates/mili-viz-client/tests/m7_render_modes.rs` — always-on
unit tests for `decode_mvg` against an `MVG3` fixture
(round-trip vertex/index/edge counts; flag-bit interpretation;
`MVG2`-fallback byte-stability); skip-on-absent end-to-end
composite render of `Translucent` and `Xray` modes against
`bar71.pltA` via `spawn_in_process`. The composite-render
fixtures land **after** [`phase-4-m7.md`](phase-4-m7.md) — until
the server emits `MVG3`, the test asserts the fallback path is
exercised and byte-stable.

## Trade-off recorded (milestone-level)

This is the smallest possible client-side change for the
volumetric story: one decoder branch, two new `RenderMode` arms,
one server round-trip toggle. The compute stays on the server
(Decision 83 — interior triangles are server-emitted); the client
keeps the thin-renderer architecture. Future polishes
(weighted-blend OIT, edge-cap LOD, per-mode `wgpu` pipeline
caching) are non-blocking additive landings.
