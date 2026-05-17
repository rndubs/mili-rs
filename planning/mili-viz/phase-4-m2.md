# `mili-viz` Phase 4 M2 — load + state navigation + real geometry (buildable scope)

> Scope doc for Phase 4 Milestone 2, the analogue of
> [`phase-4-m1.md`](phase-4-m1.md). M1 froze the wire contract and
> stood up the in-process transport with `GeometryRef` deliberately
> empty (`phase-4-m1.md` Decision 7 table). M2 attaches `mili-rs` to
> `mili-viz-server` and makes `load` / `state` / `next` / `prev` carry
> **real** mesh geometry. No proto change — the M1 contract is frozen;
> M2 is purely server-side implementation behind it.
>
> Read [`status.md`](status.md) first, then `phase-4-m1.md`. Reference
> behavior is read-only griz under `reference/griz/Src/`, cited by
> `file:line`. Decision entries continue the `phase-4-m1.md` numbering
> (Decisions 1–9 there; M2 starts at 10).

## Goal

Wire a `mili-rs` `Database` into the server so that:

- `load <root>` opens the database, and the broadcast `LoadedState`
  carries the real `num_states`, `state_times`, and element
  `class_names`.
- `state <N>` / `next` / `prev` / `first` / `last` move a cursor
  clamped to the real `[1, num_states]` bounds (griz clamps; it does
  not error on an over-range `state`).
- `show` extracts the loaded mesh — per-class triangulated index
  buffers + node coordinates **at the current state** + per-triangle
  material — and delivers it through the frozen
  `ResultState.geometry` `GeometryRef`.

Out of scope (later milestones, unchanged from `phase-4-m1.md` § "Out
of scope"): primal scalar colors (M3), selection/material geometry
effects (M4), derived results (M5), TCP/Flight-over-the-wire (M6).

## Decisions (continuing the `phase-4-m1.md` log)

### Decision 10 — bulk geometry resolves through an in-process geometry store keyed by the frozen `GeometryRef.flight_ticket`; the real Arrow-Flight `DoGet` wire is M6 (no proto change)

The frozen proto says bulk geometry rides Arrow Flight: `GeometryRef`
carries `flight_ticket` + `layout` + counts, never inline protobuf
(`phase-4-m1.md` § "Non-deltas"). Two facts shape M2:

1. The `arrow-flight` crate pins an older `tonic` than the frozen M1
   stack (`tonic` 0.14 / `prost` 0.14); co-serving a Flight
   `FlightService` next to `MiliViz` on the same in-process server
   would drag a second, incompatible `tonic` into the tree.
2. `README.md` Phase 4 M6 explicitly owns "**Same proto over gRPC +
   Arrow Flight on a TCP socket**" — Flight-over-the-wire is a named
   M6 deliverable, not M2.

**Decision: M2 keeps the frozen `GeometryRef` semantics exactly — a
real, server-assigned opaque `flight_ticket`, a `layout` string that
documents the byte schema, and real `num_vertices` / `num_indices` —
but the bytes are resolved through an **in-process geometry store** on
the server: `VizService::fetch_geometry(ticket) -> Option<Vec<u8>>`.
The in-process client and server share an address space, so the ticket
resolves directly. M6 swaps this store lookup for a real Flight
`DoGet` over TCP; the ticket, the layout, and the encoded blob are
unchanged, so M6 is a transport swap, not a contract or format
change.** This honors the contract (the ticket is meaningful, the
layout self-describes the bytes, the counts are real) while keeping
the M1 `tonic` stack single-versioned.

**Trade-off recorded.** Standing up `arrow-flight` now would make M2's
delivery path bit-identical to M6's. Rejected: it forces a second
`tonic`/`prost` major into the workspace for zero contract benefit at
M2 (the in-process client does not need the wire), and contradicts
`README.md` putting Flight-over-TCP at M6. The cost — M6 re-points
`fetch_geometry`'s callers at a `DoGet` — is small and localized
because the blob format and ticket are frozen here.

### Decision 11 — the geometry blob is a self-describing little-endian buffer; `layout = "MVG1:verts_f32x3+idx_u32+trimat_u32"`

The blob `fetch_geometry` returns is a single contiguous
little-endian buffer so M6's Flight `DoGet` can stream it verbatim and
a `wgpu` client (Phase 5) can upload slices without re-parsing
protobuf:

```
magic   : 4 bytes  = b"MVG1"
dims    : u32       = 3            (2-D meshes are padded z = 0)
n_verts : u64
n_idx   : u64                      (always a multiple of 3)
verts   : f32 * 3 * n_verts        (x,y,z per node, current state)
idx     : u32 * n_idx              (triangle list into verts)
trimat  : u32 * (n_idx/3)          (material id per triangle)
```

`GeometryRef.layout` is the stable string
`"MVG1:verts_f32x3+idx_u32+trimat_u32"`; `num_vertices == n_verts`,
`num_indices == n_idx`. Triangulation per `Superclass` (corner nodes
only; mid-side nodes of `Tet10` are ignored at M2 — the surface hull
is the corner hull, matching griz's `reference/griz/Src/faces.c`
corner-face extraction for display): `Tri`→1, `Quad`→2, `Tet`/`Tet10`
→4, `Pyramid`→6, `Wedge`→8, `Hex`→12. Lower-dimensional classes
(`Node`/`Truss`/`Beam`/`Particle`/…) contribute no triangles at M2
(line/point primitives are a Phase-5 renderer concern).

### Decision 12 — per-state node positions come from the primal `nodpos` query; an over-range `state` clamps (not errors); load does not auto-`show`

- **Per-state geometry.** Node coordinates at state *N* are the
  primal `nodpos` query at that state (the same parity-exact path
  `mili-rs` already exposes), remapped from query-label order into the
  node-array (fortran-id) order the connectivity indexes against. If
  the `nodpos` query is unavailable for a corpus, M2 falls back to the
  reference `node_coords` so `load` of any database still yields a
  drawable hull.
- **Clamping.** griz clamps the state cursor to `[1, num_states]`
  (`reference/griz/Src/interpret.c` `parse_command` → `change_state`,
  which bounds the requested state); `state 999` on a 101-state run is
  state 101, not an error. M2 clamps **only when a database is
  loaded** (known bound); with nothing loaded the M1 behavior is
  unchanged (no bound, no clamp) so the frozen M1 acceptance tests,
  which never open a real database, are unaffected.
- **No auto-`show`.** `load` broadcasts exactly one `DELTA_LOADED`
  (real `LoadedState`); `state`/`next`/… broadcast exactly one
  `DELTA_STATE`. Geometry is delivered only by `show` (one
  `DELTA_RESULT` carrying a `GeometryRef` for the current state). This
  preserves the M1 invariant "one `StateDelta` per `Execute`" that the
  frozen acceptance tests assert, and matches the request/response
  model: a client that steps state re-issues `show` to pull the new
  state's hull (griz re-renders implicitly; a server-split client
  asks). The `result` name is ignored for M2 geometry (scalar colors
  are M3); `show` with any/empty result yields the material-segmented
  hull — griz's default no-scalar view.

**Trade-off recorded.** Auto-re-broadcasting geometry on every state
step would save the client an explicit `show`, but it emits a second
`StateDelta` per `Execute`, breaking the M1 invariant the frozen gate
pins. Rejected for M2; revisit if Phase 5 shows the extra round-trip
matters (it is a client-side convenience, not a contract need).

## M2 acceptance gate

- [x] `mili-viz-server` depends on `mili-rs` (no `pyo3`/`parity`).
- [x] `load <root>` of a real corpus database populates
      `LoadedState` with the real `num_states`, `state_times`,
      element `class_names`; a non-openable root falls back to the M1
      stub `LoadedState` (frozen tests stay green).
- [x] `state`/`next`/`prev`/`first`/`last` clamp to `[1, num_states]`
      when loaded; unchanged when not.
- [x] `show` after `load` yields `ResultState.geometry =
      Some(GeometryRef)` with a real ticket, the Decision-11 layout,
      and `num_vertices > 0`, `num_indices % 3 == 0`.
- [x] `VizService::fetch_geometry(ticket)` returns the blob; it
      decodes per Decision 11 and the vertex count tracks the loaded
      mesh; the buffer differs between two distinct states (per-state
      `nodpos`) on a deforming corpus.
- [x] All six M1 acceptance tests still pass unchanged.
- [x] New test follows the CLAUDE.md skip-on-absent discipline (early
      return + `eprintln!` when the corpus fixture is absent).
      → `crates/mili-viz-server/tests/m2_geometry.rs`
      `load_state_nav_and_real_geometry`
- [x] `status.md` M2 box flipped with the gating test named;
      `README.md` open-questions table unaffected (M2 adds no proto,
      opens no design question).

## Out of scope for M2 (which milestone owns it)

- Primal scalar → vertex colors — M3 (`show <svar>` color array).
- Selection / material visibility geometry effects — M4.
- Derived results + golden-fixture validation — M5
  (`phase-4-m1.md` Decision 5).
- Real Arrow-Flight `DoGet` over TCP — M6 (Decision 10 swaps the
  in-process store for the wire; format/ticket frozen here).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 10 | Bulk geometry via in-process store keyed by the frozen `flight_ticket`; real Flight wire is M6 | M2 transport |
| 11 | Self-describing little-endian geometry blob; `MVG1` layout; per-superclass corner triangulation | M2 format |
| 12 | Per-state `nodpos`; over-range `state` clamps; `load` does not auto-`show` (one-delta invariant) | M2 semantics |
