# Feature-edge render mode — dihedral-angle "geometry only" wireframe

> **Status: 🟡 PLANNED.** Client-only follow-up to PR #82's edge
> rasterisation cleanup. Adds a new `RenderMode::FeatureEdges` that
> overlays only **silhouette / crease edges** on the shaded fill —
> the CAD-viewport look. Live status in [`status.md`](status.md).
> Decisions start at **100**.

## Why

PR #82 cleaned up *how* edges rasterise (black, screen-space line
quads, analytical AA) but kept the existing *what*: every entry in
`MVG3.element_edges` (or the `Mesh::edge_indices()` fallback) — i.e.
every cell-boundary edge on the exterior hull. For a meshed cylinder
that's every lateral subdivision of the curved wall; for a meshed
cube it's every per-face subdivision line. The result reads as
"this is a mesh" rather than "this is a cylinder".

The widely-used CAD-viewport convention shows only **feature edges**:
the outer hull's silhouette plus sharp creases. For the user's two
canonical shapes:

| Shape                  | Feature edges                                    | Filtered out                                |
|------------------------|--------------------------------------------------|---------------------------------------------|
| Meshed cube            | 12 outer cube edges (90° dihedral)               | All per-face subdivision edges (0°)         |
| Meshed cylinder (N=24) | 2N rim edges at top + bottom (90° dihedral)      | Lateral wall edges (360°/24 = 15° dihedral) |
| Meshed sphere          | (nothing — sphere has no creases)                | All subdivision edges                       |

The existing `Edges` / `Wireframe` / `Xray` modes are preserved
verbatim — `FeatureEdges` is an additive sibling.

## What lands

- `crates/mili-viz-client/src/mesh.rs`:
  - new field `Mesh::feature_edges: Option<Vec<u32>>` next to
    `element_edges` (line 14–39);
  - new fn `Mesh::compute_feature_edges(threshold_rad: f32) ->
    Vec<u32>` — pure CPU, deterministic, takes
    `(positions, indices)` and returns line-pair `u32`s in the same
    layout as `element_edges`.
- `crates/mili-viz-client/src/renderer.rs`:
  - `MeshBuffers` (line 94–105) grows a second
    `feature_edge_endpoint_buffer: wgpu::Buffer`; upload site
    (line 495–504) populates it at the same time as the existing
    edge buffer, calling `compute_feature_edges` once;
  - `edge_pipeline` (line 279–299) is reused verbatim — only the
    bound per-instance buffer differs;
  - `LINE_WIDTH_PX` is shared (no new const).
- `crates/mili-viz-client/src/shell.rs`:
  - `RenderMode` (line 129–148) gains one variant
    `FeatureEdges` — semantics: filled shaded hull (M3 pass) **+**
    feature-edge overlay using the new buffer. Routed through the
    existing `UiAction::SetRenderMode` plumbing.
- `crates/mili-viz-client/src/edges.wgsl` — **unchanged**. Same
  screen-space line-quad expansion + analytical AA, just fed a
  different instance buffer.
- New test file
  `crates/mili-viz-client/tests/feature_edges.rs`:
  - unit assertions on synthetic meshes (single hex → 12 feature
    edges; tet-meshed cube → 12 feature edges; meshed cylinder caps
    → 2N feature edges, zero lateral; meshed sphere → zero feature
    edges); deterministic — uses fixed-seed synthetic geometry, no
    fixture submodule needed;
  - composite render gate: `FeatureEdges` of `bar71.pltA` is
    byte-different from both `Shaded` and `Edges` (and stable across
    runs at sample_count=1). Follows the VB-003 pattern in
    `crates/mili-viz-client/tests/vb003_render_modes.rs`.

## Decisions

### Decision 100 — Feature edges are defined by **triangle-level dihedral angle**, not per-element-face adjacency

The user's intuition is "element-to-element face angles." Working at
the **triangle** level gives the same answer for free, because:

- A planar quad face triangulated into two triangles contributes a
  coplanar interior diagonal whose dihedral is 0° — filtered by the
  threshold without special-casing.
- The seam where two element-faces meet on the hull (e.g. two
  adjacent hex faces meeting at a 90° corner) is a triangle-pair
  boundary whose dihedral matches the element-face dihedral exactly.

So no per-element grouping is required, and the flat
`Mesh::indices: Vec<u32>` triangle list (which has no per-element
adjacency today; see `mesh.rs:14–39`) is sufficient input. This is
the load-bearing simplification that keeps the change client-only
and additive.

**Algorithm** (sort-based; O(E log E), no HashMap):

```rust
// emit (canon_edge, tri_idx, normal) for each tri-edge
for (t, [a,b,c]) in indices.chunks_exact(3).enumerate() {
    let n_t = face_normal(positions, a, b, c);
    for (u, v) in [(a,b), (b,c), (c,a)] {
        let key = (u.min(v), u.max(v));
        records.push((key, t, n_t));
    }
}
records.sort_by_key(|r| r.0);
// sweep equal-key runs:
//   len 1 → boundary, keep
//   len 2 → angle(n0, n1) > threshold ? keep : drop
//   len ≥3 → non-manifold (rare; shell-on-solid), keep
```

For a 1M-triangle mesh this is ~100 ms single-threaded; trivially
rayon-parallel if it ever becomes a hotspot.

### Decision 101 — Threshold is a **hardcoded 30°** in v1; promote to a Preferences slider only if asked

30° is the de facto default across ParaView, Blender Auto-Smooth,
and OpenSCAD; it cleanly handles the cube (90° kept), the cylinder
(15° dropped for N=24, 7.5° for N=48), and the sphere (everything
dropped). Exposing a slider on day one would (a) double the test
matrix (every threshold needs a recompute path covered) and
(b) burden the Preferences panel for a tuning knob no one has asked
for yet. The fn signature already takes `threshold_rad: f32`, so a
later Tweaks slider is a one-line wire-up.

**Trade-off recorded.** Per-mesh thresholds (cylinder N=8 wants
~50°, cylinder N=64 wants ~10°) are a niche; the 30° default
covers everything coarser than ~12 segments. A
`Preferences → Feature-edge angle` slider is the natural escalation
and lands without re-opening this milestone.

### Decision 102 — Compute **client-side, once per mesh upload**, not server-side or per-frame

The triangle list is already on the client by the time `Mesh` is
decoded; no protocol change is needed. The result is invariant
under camera, lighting, render mode, theme, scalar field, and time
within a static state — so it caches once at upload and is read
every frame at zero cost. Invalidation triggers, in order of how
often they fire:

- camera / view / colormap / theme change → **no recompute**
- render-mode toggle → **no recompute** (just bind the other
  buffer)
- threshold change (when later exposed) → recompute (~100 ms / 1M
  tris)
- new state with deformation → recompute (positions changed)
- topology change → recompute (rare)

Server-side computation was considered (it would mirror the
`MVG3.element_edges` precedent of Decision 73) but is rejected
because the threshold is a user preference — the server doesn't
know it, and pinning it to a default forfeits the slider
escalation path. A future "default-30°-precomputed-on-server"
optimisation can land additively as an opt-in MVG3 flag bit
without changing this milestone's contract.

**Trade-off recorded.** Per-state recompute on deformed meshes
(transient analyses where geometry moves frame-to-frame) costs
~100 ms / 1M tris per state. For animation playback this is
unacceptable; the natural escalation is (a) parallelise with rayon,
(b) precompute all states up front, or (c) move to the server as
the optional MVG3 section above. Non-blocking — at v1 the
visualiser is dominated by static-geometry workflows.

### Decision 103 — `RenderMode::FeatureEdges` overlays the **shaded fill**, not a clear background

Per user direction: feature edges are most useful as a CAD-style
overlay on the lit hull (silhouette plus creases over the actual
surface, so depth and shape are still visible). A "feature edges,
no fill" sibling — equivalent to today's `Wireframe` but with
feature edges instead of element edges — is **not** added in v1; it
can land as a one-line `FeatureWireframe` variant later if needed.

**Trade-off recorded.** Pure-line feature-edge mode (no fill) is
the CAD figure / publication-export use case. Punting it keeps the
v1 surface small (one new enum arm, not two) and the test matrix
manageable. Re-opening costs one variant + one composite test.

## Gating test

`crates/mili-viz-client/tests/feature_edges.rs` — always-on unit
tests over synthetic geometry (cube / cylinder caps / sphere /
single hex), no fixture submodule needed; lavapipe-gated composite
render of `FeatureEdges` against `bar71.pltA` via
`spawn_in_process`. The VB-001 byte-stability gate on `Shaded`
mode is **not touched** — feature-edge computation only runs when
the user enters `FeatureEdges` mode (or eagerly at upload, but the
default-mode rasterisation path is unchanged either way).

## Trade-off recorded (milestone-level)

This is the smallest possible client-side change for the
"geometry-only edges" story: one new `Mesh` field + one fn, one
new `RenderMode` arm, one extra GPU buffer reusing the existing
pipeline. No proto change, no shader change, no server work.
Future polishes (threshold slider, no-fill `FeatureWireframe`
sibling, rayon-parallel computation, per-state precomputation for
deformed animation, server-side MVG3 caching) are non-blocking
additive landings.
