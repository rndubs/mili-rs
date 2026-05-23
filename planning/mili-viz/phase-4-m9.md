# Phase 4 M9 — slice operator (server-side 2-D cross-section)

> **Status: 🟡 PLANNED.** Third slice of the post-MVP server feature
> batch. Requires [`phase-4-m7.md`](phase-4-m7.md). Sibling to
> [`phase-4-m8.md`](phase-4-m8.md) (cut plane keeps the kept side;
> this milestone returns **only** the 2-D slice surface). Client UI
> in [`phase-5-m9.md`](phase-5-m9.md). Decisions start at **78**.

## Why

Cut (M8) and slice are visually distinct griz/VisIt verbs and the
wireframe spec treats them as separate Rendering-menu items:

| Verb  | Geometry returned                                              |
|-------|---------------------------------------------------------------|
| Cut   | Kept-side outward boundary ∪ cap (closed; volumetric residue) |
| Slice | Cap **only** — the plane's polygonal intersection with each   |
|       | straddled element, no kept-side hull, no cap interior fill    |
|       | (a 2-D surface embedded in 3-D)                                |

The slice is the natural carrier for scalar-on-cross-section
rendering ("show pressure on the y = 0 plane"). The cap-polygon
machinery from M8 is reused verbatim; the only new piece is the
"emit cap, drop boundary" policy and the scalar-on-cap mapping.

## What lands

- The frozen `Cmd::Cutplane` gains an **additive** `slice_only:
  bool` field (`mili_viz.proto:220` extended with `optional bool
  slice_only = 8;`). This **is** a `.proto` change but it is
  field-additive (proto3 default `false` → indistinguishable from
  the M8 `cutpln` semantics for a M8-only deploy) and the
  client-server handshake (`HelloReply.compatible`) already covers
  the bump. The change is the second proto edit since the M1
  freeze (the catalog side-channel at
  [`phase-5-m4.md`](phase-5-m4.md) Decision 67 was the first — that
  one used the layout-string lever instead of a field; this one
  cannot, the cut/slice distinction is a command-semantics flag,
  not a payload tag).
- The server reuses `crates/mili-viz-server/src/clip.rs` (the M8
  module): for each straddler, it emits **only** the cap polygon
  (fan-triangulated as in M8). Kept-side boundary faces are
  dropped. All-keep and all-discard elements contribute nothing.
- Scalar mapping: when a `show <result>` is active and the user
  issues a slice, the per-vertex scalar is **interpolated** along
  the plane-edge intersection (linear along the element-edge by
  the t-parameter that placed the cap vertex). For element results
  (M5/M5b/M5c/M5d), the per-element value is taken at the
  straddler's cap centroid and broadcast to its triangle (cap
  triangles all share one centroid, so the cap reads as a single
  per-cell colour island — griz's behavior).
- `Session.slice: Option<CutPlane>` is **separate** from
  `Session.cut` (so a user can have both at once — a clipped hull
  with a slice plane drawn through the kept volume; griz allows
  this). The next geometry emit composes them: kept-side ∪ cut-cap
  ∪ slice-cap.

## Decisions

### Decision 78 — slice is an additive `slice_only: bool` on `CutPlane`, **the second** post-M1 proto change; no new `Cmd` variant

A separate `Cmd::Slice { ox, oy, oz, nx, ny, nz, relative }`
duplicates the entire `CutPlane` body and forks the dispatch
arm. An additive `optional bool slice_only = 8;` extends the
existing message by one wire byte (proto3 absent-default `false`),
is fully backward-compatible with an M8-only client (it issues
plain `cutpln`, never sets the flag, gets M8 behavior), and reuses
the M8 dispatch arm with a one-line branch (`if slice_only {
emit_cap_only() } else { emit_kept_plus_cap() }`).

The `Hello` major version bump that accompanies the proto edit is
the existing version-handshake mechanism (`phase-6-m1.md` Decision
36 — a bumped major emits `ProtocolMismatchWarning`, never an
exception). A mismatched-major client + M9 server still functions
(the server ignores `slice_only` it cannot parse and falls back to
cut semantics — proto3 unknown-field tolerance).

**Trade-off recorded.** A pure `.proto`-free implementation
("`cutpln slice` as a string-suffixed `relative` value") was
rejected — overloading a typed field with magic string semantics
is exactly the brittleness `phase-4-m1.md` Decision 1 set out to
avoid. The additive-field cost is one proto recompile and one
gating-test bump; the semantic clarity is worth it.

### Decision 79 — the cap is the **only** geometry emitted; cap interior is **not** filled with kept-side material; scalar interpolation is linear along the straddled element-edges

griz `cutpln slice` (and VisIt's slice) draw the cap as a stand-alone
2-D surface — no kept-side boundary, no "behind the slice" cells.
Per-vertex scalar interpolation is linear along each element edge
the plane intersects (the cap vertex's `t` parameter is already
computed by the marching-tables lookup; the scalar is the same
linear blend). Per-element scalars take the straddler's value and
paint the full cap-triangle fan one colour (griz fidelity — a single
cell shows a single value).

**Trade-off recorded.** Constant-per-element interpolation for
*nodal* results (per-cell flat) was rejected: it loses the
finite-element resolution the per-vertex scalar already carries.
Linear-along-edge is the same shape griz `iso_surface.c` uses for
isosurface vertex values and is parity-exact against it.

### Decision 80 — cut and slice **compose**: a single state can carry both `Session.cut` and `Session.slice`, emitted as one blob

Both verbs are session-state booleans (independent `Option<CutPlane>`
fields); the geometry pass composes them by emitting kept-side
boundary (from `cut`), cut cap (from `cut`), and slice cap (from
`slice`) into one `MVG3` blob. The slice cap rides as triangles
with a third reserved sentinel `tri_material == u32::MAX - 2`
("slice cap"), distinct from the M8 cut cap (`u32::MAX - 1`), so
the client can render the two with different translucency / colour
treatments without re-parsing intent.

**Trade-off recorded.** "Mutually exclusive — setting `slice` clears
`cut`" was rejected — composition matches griz / VisIt
expectation and costs only one extra `Option` field on `Session`.
A toolbar that wants the simpler "one or the other" semantics is a
client-side choice on [`phase-5-m8.md`](phase-5-m8.md) /
[`phase-5-m9.md`](phase-5-m9.md), not a server contract.

## Gating test

`crates/mili-viz-server/tests/m9_slice.rs::slice_operator`
— skip-on-absent against `bar71.pltA`: asserts (a) `slice_only =
true` emits **no** triangles with sentinel ∉ `{u32::MAX - 1,
u32::MAX - 2}` for straddled elements (i.e. no kept-side boundary
faces); (b) every emitted triangle lies on the plane within
`1e-5`; (c) co-existence — issuing `cutpln` then `cutpln
slice_only=true` produces one blob with both sentinels present;
(d) scalar interpolation: a `show <linear-svar>` produces cap
vertex scalars that match the analytic linear blend along the
straddled edge within `1e-5`.

## Trade-off recorded (milestone-level)

This milestone is the **first post-M1 typed-field proto change**.
The bar for that bump is high — established by the deliberate
zero-proto-change discipline of M2–M6 and the catalog side-channel
at `phase-5-m4.md` Decision 67 — and is justified here because the
alternative (string-overloading, or a duplicate `Slice` message)
both cost more than an additive boolean. After this milestone the
proto is again frozen unless another such bar is met.
