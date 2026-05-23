# Phase 4 M7 — volumetric geometry contract (`MVG3`)

> **Status: 🟡 PLANNED.** First slice of the post-MVP server feature
> batch (clip / slice / translucent / faithful internal edges). Live
> status in [`status.md`](status.md). Read this **before**
> [`phase-4-m8.md`](phase-4-m8.md) (cut-plane operator) and
> [`phase-4-m9.md`](phase-4-m9.md) (slice operator) — both consume
> the layout this milestone freezes. The companion client milestones
> are [`phase-5-m7.md`](phase-5-m7.md) (render modes consuming
> MVG3), [`phase-5-m8.md`](phase-5-m8.md), [`phase-5-m9.md`](phase-5-m9.md).
>
> Decision entries continue the **global, monotonic** log (last
> decision is 71 in [`phase-5-m4.md`](phase-5-m4.md)). This doc starts
> at **72**.

## Why

[`phase-4-m2.md`](phase-4-m2.md) Decision 11 froze the geometry blob
as a **boundary-surface representation** (`MVG1: verts + tri-indices
+ trimat`; `MVG2` adds a per-vertex scalar). That contract is
sufficient for shaded + scalar + nodal-averaged primal / derived
results — every Phase 4/5 milestone shipped against it byte-stable —
but it is **insufficient** for three known-wanted features:

1. **Faithful per-element wireframe.** `Mesh::edge_indices`
   (`crates/mili-viz-client/src/mesh.rs:195-208`) derives edges from
   the triangle list. For any superclass whose face is not already
   a triangle (Hex/Quad/Pyramid/Wedge), the per-face triangulation
   diagonal is indistinguishable from a real element edge once the
   blob has been flattened — a hex draws 12 cube edges **+ 6 face
   diagonals**, the diagonals slice each face into "triangles" and
   the wireframe reads as a triangulated soup rather than a cube
   grid. See [`bug-tracker.md`](bug-tracker.md) VB-005.
2. **Clip / slice / "show wedges taken out".** The server has no
   element connectivity in the blob — only an outward-face hull —
   so a clip plane has nothing to intersect against. The frozen
   `Cmd::Cutplane` arm (`crates/mili-viz-server/src/lib.rs:528`) is
   a stub for exactly this reason.
3. **Translucent whole-mesh / "X-ray" rendering.** The hull pass
   emits only outward faces; cell-cell interfaces between adjacent
   solid elements are dropped. Drawing the mesh translucent shows
   the silhouette but **no internal structure** — there are no
   interior triangles in the blob.

A faithful fix touches the blob contract, the server-side hull
extractor, and the client decoder. Doing it as one frozen layout
revision (M7) lets M8/M9 (cut plane / slice operators) and
[`phase-5-m7.md`](phase-5-m7.md) (render modes) ship without
re-opening the geometry seam each time.

## What lands

- A new self-describing blob layout, `MVG3`, that is a **strict
  superset** of `MVG2`. Layout tag goes in
  `GeometryRef.layout` (already a free-form string — **zero
  `.proto` change**, same backward-compatibility lever the catalog
  side-channel used at `phase-5-m4.md` Decision 67). Old clients
  decoding `MVG3` see a recognized magic and a length they can
  parse to skip the new trailing sections (the layout is fully
  length-prefixed); new clients use them.
- The server's hull extractor (`crates/mili-viz-server/src/geometry.rs`
  `MeshTopology::build`) gains a **per-superclass element-edge
  table** alongside `triangulation()`; the build appends to a
  global `element_edges: Vec<u32>` (line-list, undirected, unique
  per element) so the wireframe pass on the client reads true mesh
  lines, not triangle legs.
- The same build gains an opt-in **interior-triangle** pass
  (controlled by a viz-state `RenderMode` mirror — Decision 73):
  when on, every cell-cell shared face is emitted **once** with an
  `interior=1` flag bit so the client can draw it translucent
  without double-counting the boundary.
- A per-triangle `flags_u32` column joins the existing
  `tri_material_u32` column. Bit 0 = `interior`; bits 1–31 reserved.
- The client decoder (`crates/mili-viz-client/src/mesh.rs::decode_mvg`)
  learns the `MVG3` magic, decodes the new sections, and exposes
  them as `Mesh::element_edges` and `Mesh::tri_flags`.
  `Mesh::edge_indices` is **superseded** by the server-supplied
  buffer (the on-the-fly triangle-edge extraction stays as the
  `MVG1`/`MVG2` fallback so the existing render path is byte-stable
  when an `MVG3`-unaware server is connected).

## Blob layout (`MVG3`)

Little-endian, length-prefixed every section, all offsets implied:

```
magic         : 'M' 'V' 'G' '3'    // 4 bytes
dims          : u32 = 3
n_verts       : u64
n_idx         : u64                 // multiple of 3
n_edges       : u64                 // multiple of 2 (line-list)
flags_mask    : u32                 // bitfield of present sections:
                                    //   bit 0 = scalar_f32 (MVG2-compat)
                                    //   bit 1 = tri_flags_u32
                                    //   bit 2 = element_edges_u32
                                    //   bit 3 = interior tris included
verts         : f32 * 3 * n_verts
indices       : u32 * n_idx
tri_material  : u32 * (n_idx / 3)
[ tri_flags  : u32 * (n_idx / 3) ]  // iff bit 1
[ edges     : u32 * n_edges ]       // iff bit 2; line-list
[ scalar   : f32 * n_verts ]        // iff bit 0
```

`n_edges` is allowed to be 0 (no element-edge buffer; the client
falls back to triangle-edge derivation, matching `MVG1`/`MVG2`
behavior). `tri_flags` is absent iff `flags_mask & 2 == 0`.

## Decisions

### Decision 72 — `GeometryRef.layout = "MVG3"` is the additive successor to `MVG2`; superset, length-prefixed, zero `.proto` change

`GeometryRef.layout` is a free-form string by construction
(`phase-4-m1.md` Δ1; the field has been a tag, not an enum, since
the frozen contract). `MVG2` is a strict subset of `MVG3` (set
`flags_mask = 0b0001` and omit the trailing sections — byte-identical
to today's `MVG2` modulo the eight-byte header expansion for
`n_edges`/`flags_mask`). The server emits `MVG3` only when the new
fields would be non-trivial (interior triangles present, or the
element-edge buffer requested); otherwise it stays on `MVG2` and the
M2/M3/M4 composite gates are byte-stable.

**Supersedes** [`phase-4-m2.md`](phase-4-m2.md) Decision 11 (the
`MVG1`/`MVG2` exhaustive layout list) **additively** — `MVG1` and
`MVG2` remain valid and the client keeps decoding them. The fixture
gates against `MVG1`/`MVG2` stay byte-stable.

**Trade-off recorded.** A typed `oneof Layout { MVG1 m1=1; MVG2 m2=2;
MVG3 m3=3; }` was rejected: it forces a `.proto` change (and a
co-deployed client/server bump) for every future layout. The
free-form tag is the same backward-compat lever Decision 67 used for
the catalog blob and Decision 11 reserved by leaving `layout` a
string from day one.

### Decision 73 — element-edge buffer is **server-derived from the per-superclass edge table**, not extracted from triangles

The viz-local table mirrors `triangulation()`
(`crates/mili-viz-server/src/geometry.rs:76-118`): a fixed
`[[usize; 2]]` per superclass enumerating the **element's true
edges** (Hex = 12, Tet = 6, Quad = 4, Tri = 3, Wedge = 9,
Pyramid = 8, Tet10 = 6 corner edges only — mid-edge subdivision
is a follow-up). For each element the server appends `(node_a,
node_b)` pairs as `u32`s into `MVG3.edges`. Deduplication is
**per-element only** (the cheapest correct policy — a hex still
draws each cube edge once because the table has 12 unique pairs;
two hexes sharing a face draw the shared edge twice on top of
itself, which is visually identical to once and avoids a
global-hash pass). A global hash dedupe is a future polish if
edge-count growth becomes a bandwidth issue on M-element corpora.

**Discharges** [`bug-tracker.md`](bug-tracker.md) VB-005 (hex face
diagonals appear in `Edges`/`Wireframe` modes): the client uses
`MVG3.edges` when present and the diagonals never enter the wire
pass.

**Trade-off recorded.** Triangle-edge extraction + a "this edge is
the splitting diagonal" predicate was considered and rejected: the
predicate needs full quad-face reconstruction at the client (fragile
under deformation; impossible without per-superclass knowledge the
hull blob deliberately erased). The server table is one constant
per superclass — the same shape `triangulation()` already is — and
ships the answer in the blob.

### Decision 74 — interior triangles are emitted opt-in via a viz-state `IncludeInterior` flag broadcast through the existing `MaterialsState` extension, **no `.proto` change**

`MaterialsState` (`mili_viz.proto:356`) already carries `map<u32,
bool> visible` (per-material toggle) as the canonical per-class
visibility state. The "include interior" flag is the same shape — a
session-state boolean that the next `show` consults — and rides as
a reserved sentinel key in `MaterialsState.visible` (`u32::MAX` →
interior on/off). The client lowers a `Rendering → Translucent`
toggle to a `Cmd::Material(MaterialVisibility{ enable: bool,
class_name: "", material: Some(u32::MAX) })` — a verb the proto
already accepts, repurposed for a viz-only concern.

When the flag is on, `MeshTopology::build` walks the per-class
connectivity and emits **every face** (not just outward boundary
faces) with `tri_flags |= 1` set for interior faces. The hull
discovery already needs the face → adjacent-cell map; flipping the
emit policy is a one-line change at the gather. The blob grows
proportional to the interior face count (~5× boundary triangles for
a dense hex corpus); the server **only** does this when the client
asks.

**Trade-off recorded.** Adding a typed `Cmd::IncludeInterior { bool
on }` was rejected: it bumps `.proto`, forces a client/server
co-deploy, and replicates state already expressible through
`MaterialVisibility`. The reserved-sentinel key is the same trick
the catalog blob used (`CATALOG_TICKET = b"catalog:current"`,
`phase-5-m4.md` Decision 67) — a constant the existing field
encodes, no new field needed. A clean typed cmd is an additive
proto polish if the sentinel pattern proves opaque.

## Gating test

`crates/mili-viz-server/tests/m7_mvg3.rs::volumetric_geometry_contract`
— asserts: (a) round-trip encode→decode of an `MVG3` blob with all
four flag bits set; (b) hex element emits exactly 12 element edges
(no face diagonals) and the edge endpoints match the corner table;
(c) `IncludeInterior` on a two-hex cube emits one interior quad
(two triangles, `flags & 1 == 1`) above the boundary count;
(d) `MVG2` decode path stays byte-identical (composite-byte-stable
vs M2/M3/M4/M5/M5b/M5c/M5d/M6 fixture goldens).

## Open questions for M7

- **`Tet10` mid-edge nodes in the edge buffer.** Decision 73's
  table is corner-only (6 edges). Drawing the mid-edge segment
  (12 sub-edges per Tet10) is a follow-up — the layout has room
  (each `Tet10` element can contribute up to 12 edge pairs without
  any blob-format change).
- **Surface superclass.** The viz currently triangulates Surface
  identically to Quad/Tri at `triangulation()`; the edge table
  follows the same pattern. No special-case here.

## Trade-off recorded (milestone-level)

Doing this as one frozen contract — versus three independent blob
revisions for clip / slice / translucent — costs one extra layout
tag now but pays back at M8/M9/Phase-5-M7+: each of those
milestones ships with **zero geometry-contract movement**. The
existing skip-on-absent fixture/parity discipline still applies
end-to-end (any test that runs the new path is skip-on-absent until
the corpus exercises it).
