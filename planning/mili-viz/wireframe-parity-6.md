# Wireframe-parity #6 — picking class-N label (path (a))

Closes the last picking gap from
[`wireframe-parity.md`](wireframe-parity.md) row #6 and the
cross-cutting "Picking (client-side from cached `GeometryRef`)" row:
the status-bar readout now resolves to legacy griz `<class> <label>`
form (`brick 42 · v=…`) instead of the placeholder
`tri T · node N · v=…`.

The frozen `mili_viz.proto`'s `GeometryRef` carries no per-element
label catalog. Two paths were on the table (see
[`wireframe-parity.md`](wireframe-parity.md) "What's still left" #6):

- **(a)** Catalog side-channel + per-tri owner id, zero per-pick
  latency, one-time blob bloat at load.
- **(b)** `Query` round-trip per pick, zero load-time cost, one
  round-trip per pick.

The maintainer picked **(a)** — pure-local resolve, no new wire RPC.

## What landed

- Server (`mili-viz-server`):
  - `MeshTopology::build_volumetric_faces` returns a fourth parallel
    column `tri_member_id: Vec<u32>` alongside `indices` /
    `tri_material` / `tri_flags`.
  - `MeshTopology::encode_mvg3` funnels the column through the
    existing material-visibility / interior-bit filter
    (`geometry.rs:1153-1170`) and serialises it at the tail behind a
    new `flags_mask` bit 4.
  - `pack_mvg3_buffers` takes an optional `tri_member_id: Option<&[u32]>`;
    when present, sets bit 4 and emits the trailing column.
  - `clip::ClipBuffers` gains a parallel `tri_member_id` field.
    Kept triangles inherit the source element's packed id; cap
    triangles (cut/slice intersections with no single owning
    element) push the sentinel `TRI_MEMBER_NONE = u32::MAX`.
  - `Session::catalog_blob` appends one `M\t<class_idx>\t<class_name>
    \t<labels.csv>` row per non-empty element class. `class_idx`
    matches the high 8 bits of the geometry blob's `tri_member_id`
    because both walks iterate `MeshTopology::elem_classes` in
    build-order.
- Client (`mili-viz-client`):
  - `Mesh` carries optional `tri_member_id: Option<Vec<u32>>`;
    `decode_mvg3` reads it when `flags_mask & 16 != 0`.
  - `Pick` carries optional `member_id: Option<u32>`; `Mesh::pick`
    populates it from `tri_member_id[tri]`, filtering the sentinel
    to `None`.
  - `ResultCatalog` carries `classes: Vec<ClassMembership>` parsed
    from the catalog blob's new `M` tag; `resolve_member(id)` unpacks
    `(class_idx, elem_row)` and returns `(class_name, label)`.
  - `ShellState::apply_pick` uses the catalog when a `member_id`
    resolves, formatting `<class> <label> · v=…`; otherwise falls
    back to the legacy `node N · tri T · v=…` form so older servers
    or cap-tri picks still print something useful.
- Wire format: bit 4 in MVG3 `flags_mask`, new `M\t` tag on
  `MVCAT1`. **No `.proto` edit.** Forward-compatible in both
  directions (decoder ignores unknown bits/tags by construction).

## Decisions

### Decision 104 — Per-tri owner id lives on the geometry blob, class table on the catalog

`encode_mvg3` rewrites the triangle list per frame (material
visibility + interior-bit filter at `geometry.rs:1153-1170`), and
`clip::clip_topology` synthesises an entirely new triangle set under
cut/slice. A pure-catalog "class owns triangle range" mapping would
either bind to *unfiltered* indices (forcing the client to mirror the
filter) or break under cut/slice. Putting the per-tri owner id on
the geometry blob makes the pick-time lookup sample-perfect under
both filtering paths, at the cost of one extra `u32` per visible
triangle (~4 bytes/tri; for a 50k-tri hull, ~200 KB per blob —
negligible on the wire compared to the existing vertex/index
columns). The small `class_idx → (class_name, labels[])` table stays
on the catalog side-channel — one-time at load, ≤a few KB per class.

### Decision 105 — `class_idx << 24 | elem_row` packing, 8/24 split, `u32::MAX` sentinel

Packs `(class_idx, elem_row)` into a single `u32` to avoid widening
the per-tri column to 8 bytes. 8-bit class index caps at 256 (no
griz corpus has anywhere near that many element classes); 24-bit
element row caps at ~16M (well beyond any d3samp / xmilics fixture).
The reserved sentinel `u32::MAX` ("no owning element") is used for
cut/slice cap tris and surfaces as `Pick::member_id = None` so the
status bar gracefully falls back to the legacy `tri T  node N`
readout for caps. `pack_tri_member_id` asserts the bounds so a
future corpus that exceeds them fails loudly rather than silently
truncating.

### Decision 106 — Catalog `M\t<class_idx>\t<class_name>\t<labels.csv>` tag, same `MVCAT1` magic

Stays under the existing magic (no version bump) because the client's
`decode_catalog` already tolerates unknown tags
(`catalog.rs:73-77` — the existing forward-compat seam reserved for a
future `T` time-indep tag). Element labels are comma-separated
decimal `i32` — verbose but human-readable and ~a few KB per class
in any realistic corpus. Malformed `M` rows drop silently
(consistent with the rest of the catalog's tolerance posture).
`class_idx` is dense from 0 in `MeshTopology::elem_classes` build
order so the high 8 bits of `tri_member_id` index directly into
`ResultCatalog::classes[class_idx == class_idx]`.

## Gating tests

Server:
- `crates/mili-viz-server/src/geometry.rs::tests::two_hex_cube_volumetric_dedup_marks_one_interior_face`
  asserts the per-tri member ids match the dedup attribution
  (first-encountered element wins on the shared face).
- `tests/m7_mvg3.rs` asserts bit 4 round-trips, the column is
  parallel to triangles, no cap sentinel in the base hull,
  `class_idx` starts at 0.
- `tests/m8_cutplane.rs` asserts cap tris carry the sentinel and
  kept tris carry valid packed ids.
- `tests/catalog.rs` asserts each `M` row parses with the expected
  shape, `class_idx` is monotone, labels are non-empty.
- The five existing MVG3 decoders in `m2_geometry.rs`, `m3_primal.rs`,
  `m4_visibility.rs`, `m5_derived.rs`, `m5b_principal.rs`,
  `m5c_derived.rs`, `m5d_alt_strain.rs`, and `m6_transport.rs` all
  walk past the new column so the blob is fully consumed (forward-
  compat sanity).

Client (`crates/mili-viz-client/tests/picking.rs`, 5 new always-on
cases):
- `pick_carries_member_id_when_geometry_blob_does` — pick on a
  member-tagged quad surfaces the right packed id per tri.
- `pick_omits_member_id_when_blob_has_no_column` — old blob → `None`.
- `pick_omits_member_id_for_cap_sentinel` — cap sentinel → `None`.
- `catalog_resolve_member_unpacks_class_and_label` — `resolve_member`
  unpacks and looks up labels.
- `shell_apply_pick_uses_catalog_when_member_resolves` — readout
  formats `<class> <label> · v=…`.
- `shell_apply_pick_falls_back_when_catalog_lacks_member` — empty
  catalog → legacy `node N · tri T` form.
- `decode_catalog_parses_m_tag_and_tolerates_unknown_tags` — `M`
  parses, `Z` unknown tag drops silently.
- `decode_catalog_drops_malformed_m_rows` — every malformed shape
  drops.
- `mvg3_blob_round_trips_member_id_column` — hand-built MVG3 with
  bit 4 set decodes the column round-trip; sentinel surfaces as
  `None` in `Pick::member_id`.

## Out of scope (follow-up)

- The picking-driven variant of the Plot tab (click element →
  "+series" for that element's time-history) is now unblocked but
  not implemented here. The new `Pick::member_id` + `ResultCatalog::
  resolve_member` is the right plumbing to key it off; the Plot tab
  text-input variant remains the supported form for now.
- TI-results catalog (the reserved `T` tag in the catalog blob)
  remains blocked on a `mili-rs` core TI accessor — untouched.
