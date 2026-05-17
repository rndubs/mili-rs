# `mili-viz` Phase 4 M4 — selection + enable/disable (buildable scope)

> Scope doc for Phase 4 Milestone 4, continuing
> [`phase-4-m3.md`](phase-4-m3.md). M2 delivered the per-state
> triangulated hull; M3 added an optional per-vertex scalar. M4 makes
> **material visibility** actually filter the emitted geometry and
> pins **selection** as a contractual broadcast (not a hull edit). No
> proto change — the M1 contract is frozen; the geometry blob is
> server-internal and self-describing (the `GeometryRef.layout` string
> already versions it; `num_indices` already reports the triangle
> count).
>
> Read [`status.md`](status.md) first, then `phase-4-m3.md`. Reference
> behavior is read-only griz under `reference/griz/Src/`, cited by
> `file:line`. Decisions continue the log (M1: 1–9; M2: 10–12;
> M3: 13–15; M4 starts at 16).

## Goal

After a `load`:

- `enable`/`disable` (`MaterialVisibility`, `DELTA_MATERIALS`) makes a
  material invisible: triangles of a disabled material are **excluded
  from the geometry blob** on the next `show`. The blob already carries
  per-triangle material (M2 Decision 11), so the filter is a triangle
  pass — it applies identically to the `MVG1` bare hull and the `MVG2`
  scalar hull (the per-vertex scalar array is untouched).
- `select`/`clrsel` (`Select`/`ClearSelection`, `DELTA_SELECTION`) is
  tracked and broadcast through the existing `SelectionState` delta and
  the late-joiner `Snapshot` (M1 already does this), matching griz's
  non-destructive overlay model — selection does **not** edit the hull.

Material visibility / selection take effect on the next `show`,
consistent with M2 Decision 12 / M3's request–response model: one
`StateDelta` per `Execute`, the M1 frozen-acceptance invariant.

Out of scope (unchanged from `phase-4-m3.md`): derived results (M5),
Flight-over-TCP (M6), the client highlight/colormap rendering
(Phase 5).

## Decisions (continuing the log)

### Decision 16 — `enable`/`disable` filters the emitted triangle list by per-triangle material; absent/`true` = visible; the per-vertex scalar array and its `(min,max)` range are unchanged

griz's display-face extraction drops faces whose element's material is
hidden (`reference/griz/Src/faces.c:2031` for hex,
`Src/faces.c:1883` for tet — `if ( hide_mtl[materials[i]] ) …` skips
the face). The M2 blob already stores a per-triangle material id
(`tri_material`, M2 Decision 11), so the server has the exact key griz
filters on.

**Decision: a material is visible unless `Session.materials` maps it to
`false` (the M1 `MaterialVisibility` tracking is unchanged — `enable`
sets `true`, `disable` sets `false`; a material never named stays
visible). On `show`, a triangle is emitted only if its material is
visible. The filter is a single pass over `(indices, tri_material)`
applied **before** encoding, so it composes identically with `MVG1`
(no scalar) and `MVG2` (scalar present): the per-vertex scalar array,
its length (`num_vertices == node_count`), and the reported
`ResultState.{min,max}` are byte-for-byte what M3 produced — only the
triangle list (`indices` / `tri_material`) and therefore
`num_indices` shrink. `num_vertices` stays the full node count
(unreferenced vertices are exactly the M3 untouched/`NaN` case — a
client renders them as nothing). The blob magic and `layout` string
are unchanged (`MVG1`/`MVG2`); the buffer is still fully
self-describing and `num_indices` already reports the post-filter
triangle count — so this is not a format change and the frozen M2/M3
tests, which never disable a material, get a byte-identical blob.**

`MaterialVisibility.class_name` and a `None` `material` stay
no-ops for the geometry filter (the M1 behavior: only a `Some(material)`
mutates `Session.materials`). griz's `disable_material` array is keyed
by global material number, not by class; the blob's per-triangle key is
likewise the material id, not a class. Class-scoped material toggles
would need a class→material map the M2 topology does not carry.

**Trade-off recorded.** griz also recomputes result autoscale over the
*visible* set, and additionally distinguishes `disable` (drawn grey,
excluded from result coloring) from `invis` (removed entirely). M4
collapses both onto **exclusion from the blob** and leaves the M3
scalar/range untouched. Rejected the griz-exact split because: (a) the
server's contract output is geometry + scalar + data range, not a
render style — greying a still-present material is a Phase-5 client
decision over per-triangle material the blob already carries (parallel
to M3 Decision 15: the legend clamp is the renderer's); (b)
re-deriving the nodal-averaged scalar excluding hidden-material
elements would re-litigate the M3 blob format for one milestone's
benefit (parallel to M3 Decision 14's per-element rejection); the
range is a client-side legend clamp anyway (M3 Decision 15). The cost
— a hidden material's nodes still influence the reported `(min,max)` —
is a clamp the client already owns. Exclusion (not greying) is the
minimal contract-honest change that genuinely edits the emitted
geometry, which is what the milestone asks for.

### Decision 17 — selection stays metadata-only: broadcast via the existing `DELTA_SELECTION` `SelectionState` and the `Snapshot`; no blob change; `clrsel` with an empty class clears the whole selection

griz `select` (`reference/griz/Src/interpret.c:1081`) and `clrsel`
(`Src/interpret.c:1450`, aliased `poof`) maintain a highlighted/picked
set drawn as a **non-destructive overlay** — `select` never removes or
rewrites mesh faces, unlike `hide_material`. The frozen proto already
carries selection as first-class session state: `DELTA_SELECTION` →
`SelectionState{by_class}` (M1 tracks the map; `apply` already
broadcasts it) and the late-joiner `Snapshot.selection`.

**Decision: selection is reflected in the contract output through the
existing `SelectionState` delta + `Snapshot`, exactly as M1 wired it —
the geometry blob is unchanged (no `sel_u32` channel, no `MVG3`). A
Phase-5 client highlights selected elements client-side from
`SelectionState` plus an element-connectivity `Query`, mirroring M1
Decision 2 (picking is client-side from cached geometry + `Query`, no
new proto) and griz's overlay model. One refinement for griz fidelity:
`clrsel` with an empty `class_name` clears the entire selection map
(griz `clrsel`/`poof` clears all selected objects); a named class
clears just that class (the M1 behavior).**

**Trade-off recorded.** A per-triangle selected mask in the blob
(a new `MVG3` layout) would let a client highlight without a `Query`
round-trip. Rejected: it requires an element-id-per-triangle channel
the M2 format deliberately omitted (the blob is vertex-indexed with no
element identity), explodes the layout matrix (scalar × selection
combinations), and bloats every `show` for an overlay griz itself
keeps out of mesh topology. Metadata-only is the minimal
contract-honest representation and is consistent with M1 Decision 2 /
M3 Decision 15 (the server reports state and data; overlay and
rendering are the client's). If Phase 5 shows the extra round-trip
hurts, a selection mask is a purely additive future layout, not an M4
contract debt.

### Decision 18 — material/selection changes take effect on the next `show`; one `StateDelta` per `Execute` preserved; no proto/format change (README open-questions unaffected)

**Decision: `enable`/`disable`/`select`/`clrsel` each broadcast
exactly their own one delta (`DELTA_MATERIALS` / `DELTA_SELECTION`) and
do **not** emit a geometry `StateDelta`; the visual effect lands when
the client re-issues `show` (which re-encodes the current state's hull
with the live visibility filter). This preserves the M1 frozen
"one `StateDelta` per `Execute`" invariant (M2 Decision 12, the
request–response model: a client that mutates view state re-issues
`show` to pull the updated hull). No proto field, no blob magic, no
`layout` string changes — M4 is server-side only, so `README.md`'s
open-questions table is unaffected (no proto change, no new design
question).**

## M4 acceptance gate

- [x] `disable <_> <mat>` then `show` after `load` yields a blob whose
      `num_indices` is strictly smaller than the all-visible `show`
      (the disabled material's triangles are gone), the surviving
      `tri_material` values exclude the disabled id, and
      `num_vertices` is unchanged.
- [x] The filter composes with `MVG2`: `show <element-scalar>` with a
      material disabled stays `layout == "MVG2:..."`, the scalar array
      still has `num_vertices` entries, and `ResultState.{min,max}`
      bracket the finite values (M3 path byte-stable).
- [x] `enable <_> <mat>` restores the material: `num_indices` returns
      to the all-visible count; the blob is byte-identical to the
      pre-disable `show` at the same state.
- [x] `select`/`clrsel` broadcast exactly one `DELTA_SELECTION` each
      with the expected `SelectionState.by_class`; `clrsel` with an
      empty class empties the map; a fresh subscriber's `Snapshot`
      carries the live selection. The geometry blob is unchanged by
      selection.
- [x] Each of `enable`/`disable`/`select`/`clrsel` emits exactly one
      `StateDelta` per `Execute` (M1 invariant); the kinds match
      `command_delta_kind` (`DELTA_MATERIALS` / `DELTA_SELECTION`).
- [x] All six M1 acceptance tests + `m2_geometry.rs` + `m3_primal.rs`
      still pass unchanged (no material disabled → byte-identical
      blobs).
- [x] New test follows the CLAUDE.md skip-on-absent discipline (early
      `return` + `eprintln!` when the corpus fixture is absent).
      → `crates/mili-viz-server/tests/m4_visibility.rs`
      `material_visibility_and_selection`
- [x] `status.md` M4 box flipped with the gating test named;
      `README.md` open-questions table unaffected (no proto change).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 16 | `enable`/`disable` filters the emitted triangle list by per-triangle material; absent/`true` = visible; scalar array + `(min,max)` unchanged; composes identically with `MVG1`/`MVG2` | M4 material visibility |
| 17 | Selection stays metadata-only (existing `DELTA_SELECTION` + `Snapshot`, no blob change); `clrsel` empty class clears all | M4 selection |
| 18 | Effects apply on the next `show`; one `StateDelta` per `Execute`; no proto/format change | M4 semantics |
</content>
</invoke>
