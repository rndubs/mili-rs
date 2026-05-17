# `mili-viz` Phase 4 M5 follow-up (third slice) — surfstrain + nodal-time derived families (buildable scope)

> Scope doc for the **third and final M5-family sub-slice**, continuing
> [`phase-4-m5b.md`](phase-4-m5b.md). M5 routed the scalar stress
> invariants; M5b added the eigenvalue families; both reuse the
> already-prepped-element-class scatter seam. This slice closes the two
> remaining gathers `phase-4-m5b.md` Decision 22 explicitly deferred —
> the **nodal time-derived families** (node-direct gather) and
> **`surfstrain{x,y,z,xy,yz,zx}`** (per-face Hex connectivity gather) —
> and **re-defers `*_alt`** with a recorded rationale (no parity-exact
> `mili-rs` kernel exists for it). No proto change; server-side only;
> the M5/M5b/M3/M6 paths stay byte-stable.
>
> Read [`status.md`](status.md) first, then `phase-4-m5.md`
> Decisions 19–21 and `phase-4-m5b.md` Decisions 22–24 — the derived
> validation philosophy this doc continues **verbatim**: reuse the
> already-parity-exact `mili-rs` kernels (no formula re-port, no griz
> golden, **no `parity` feature in `mili-viz-server`**); the core
> parity suite owns kernel numerics, the viz gating test owns only the
> *routing*; gating invariants ride a **single shared gather** only.
> Decisions continue the log (M1: 1–9; M2: 10–12; M3: 13–15; M4:
> 16–18; M5: 19–21; M5b: 22–24; M6: 25–27; this slice starts at **28**).

## Goal

`show <name>` after a `load`, for the families:

- nodal time-derived: `disp_x` / `disp_y` / `disp_z`, `disp_mag`,
  `disp_rad_mag_xy`, `vel_x` / `vel_y` / `vel_z`, `acc_x` / `acc_y` /
  `acc_z`
- per-face Hex surface strain: `surfstrainx` / `surfstrainy` /
  `surfstrainz` / `surfstrainxy` / `surfstrainyz` / `surfstrainzx`

routes through the **already-parity-exact** `mili-rs` kernels
(`mili_rs::compute_node_{displacement,displacement_magnitude,velocity,
acceleration}` + `nodal_reference_from_coords`; `mili_rs::Database::
surface_strain_query`) into the geometry blob, keeping the `MVG1`/
`MVG2` blob format, `flight_ticket`, and `ResultState.{min,max}`
autoscale byte-stable for every prior path. Unknown / unresolvable
names, class-absent, or query-failure still fall back to the M3 bare
hull (the "`show` is total" invariant from M3 Decision 13 / M5
Decision 20 holds — `show` never errors).

Out of scope (re-deferred — see Decision 28): the `*_alt` griz
closed-form trig principal-strain variants
(`prin_strain[1-3]_alt` / `prin_dev_strain[1-3]_alt`). Flight-over-TCP
is M6 (landed, untouched).

## Decisions (continuing the log)

### Decision 28 — this slice's family set is exactly the nodal time-derived families + `surfstrain{x,y,z,xy,yz,zx}`; the `*_alt` trig principal-strain variants are **re-deferred** because no parity-exact `mili-rs` kernel exists for them (routing them would breach M5 Decision 19's "reuse the kernel, no formula re-port" boundary)

`phase-4-m5b.md` Decision 22 deferred three things as "a different
gather": `surfstrain*`, the `*_alt` trig strains, and the nodal
time-derived families. Auditing `crates/mili-rs/src/derived.rs` and
`crates/mili-rs/src/geometry.rs` against
`reference/mili-python/src/mili/derived.py` shows they are **not** in
the same state:

- **Nodal time-derived** — the parity-exact kernels exist and are
  public: `compute_node_displacement`, `compute_node_displacement_
  magnitude`, `compute_node_velocity`, `compute_node_acceleration`,
  `nodal_reference_from_coords`, `node_{disp,disp_mag,vel,acc}_spec`,
  `node_disp_primal` (`derived.rs:41-257,259-279,620-701`), driven
  today by `crates/mili-py` (`database.rs` ~1138–1312). Resolvable.
- **`surfstrain*`** — the parity-exact kernel exists as a `Database`
  method: `mili_rs::Database::surface_strain_query` (`geometry.rs:726`,
  `derived.py.__compute_surface_strain`), with `surfstrain_spec`
  (`derived.rs:1423`) selecting the `(title, jr, ic)` component. It is
  driven today by `crates/mili-py` (`database.rs:1619-1662`).
  Resolvable, but only through a **per-face Hex connectivity gather**
  (the kernel takes a mandatory `face ∈ 1..=6`), which is exactly the
  "different gather than 'the invariant lives wherever `sx`/`ex`
  lives'" Decision 22 called out.
- **`*_alt`** — **no kernel exists in `mili-rs`.**
  `__compute_principal_strain_alt` / `__compute_dev_principal_strain_alt`
  (`derived.py:1219-1408`) are present only in the Python reference;
  `derived.rs:29` and `derived.rs:1226-1227` explicitly mark the
  `*_alt` griz closed-form trig variants as "a later sub-slice", and
  `crates/mili-py` does **not** route them either. There is nothing
  parity-exact to reuse.

**Decision: this PR implements precisely the nodal time-derived
families (`disp_x/y/z`, `disp_mag`, `disp_rad_mag_xy`, `vel_x/y/z`,
`acc_x/y/z`) and `surfstrain{x,y,z,xy,yz,zx}` via the parity-exact
`mili-rs` kernels. `prin_strain[1-3]_alt` / `prin_dev_strain[1-3]_alt`
are re-deferred** — routing them would require either (a) re-porting
`__compute_principal_strain_alt` into `mili-viz-server`, which
directly violates M5 Decision 19's load-bearing boundary ("reuse the
already-parity-exact kernel, **no** formula re-port, **no** `parity`
feature in `mili-viz-server`" — the invariant the whole M5 family is
built on), or (b) first adding a new `mili-rs` core kernel **plus** its
Python-oracle parity validation under the `parity` feature — a
`mili-rs` **core** sub-slice (the analogue of the Phase-H derived
sub-slices), not a server-side viz **producer** slice like M5/M5b/this.

This also **corrects an imprecision in Decision 22**, which described
the nodal time-derived families as "already reachable as primals via
the M3 path". That is true only of the *underlying* `nodpos`
(`ux`/`uy`/`uz`) primal; the *derived names* (`disp_x`, `vel_x`,
`acc_x`, …) are **not** primals, so `classes_of_state_variable
("disp_x")` resolves nothing and M3 falls to the bare hull — they need
the explicit routing this slice adds.

**Trade-off recorded.** Including `*_alt` would "finish strain" in one
PR. Rejected: it is the one deferred family with **no** parity-exact
kernel to reuse, so any in-slice implementation contradicts the single
most load-bearing M5-family invariant (Decision 19); the correct home
is a future `mili-rs` core derived sub-slice that ports the trig
variant *and* parity-validates it against the `mili` Python package,
after which a trivial follow-up routes it here through this exact seam.
The cost — `*_alt` stays unreachable in viz until that core slice — is
bounded and explicitly the right division of labor (core owns kernel
numerics; viz owns routing).

### Decision 29 — nodal-time routing: a node-direct branch group mirroring the `crates/mili-py` `query()` nodal dispatch for the single current state, reusing M3's node→vertex mapping (factored, the M3 primal nodal path byte-stable); element scatter untouched

The nodal time-derived families are **nodal** results (node class),
computed from the `nodpos` components across one or more states. They
do not fit the M5/M5b element-class scatter — they fit M3's existing
*nodal-field* branch (`classes.iter().any(|c| c == "node")` → map node
label → vertex directly, `geometry.rs:418-430`).

**Decision: add a branch group to `MeshTopology::vertex_scalar`,
immediately after the M5b `principal_strain_spec` branch and before
the primal `classes_of_state_variable(svar)` lookup, that resolves via
`node_disp_spec` / `node_disp_mag_spec` / `node_vel_spec` /
`node_acc_spec` and mirrors the `crates/mili-py` `query()` nodal
dispatch (`database.rs` ~1138–1312) for the *single current state*:**

- `disp_*` / `disp_mag` / `disp_rad_mag_xy`: `query_full` the
  `node_disp_primal(dir)` (`ux`/`uy`/`uz`) on the `node` class at the
  current state; build the per-label reference via
  `nodal_reference_from_coords` at **`reference_state == 0`** (the
  upstream default and the only value the corpus exercises; a non-zero
  reference is an upstream-rejected extension, never a silent wrong
  answer, and the viz `show` vocabulary has no reference-state arg);
  call `compute_node_displacement` / `compute_node_displacement_
  magnitude`.
- `vel_*` / `acc_*`: gather `node_disp_primal(dir)` at exactly the
  stencil states the kernel needs (velocity → `{s, s-1}`;
  acceleration → forward `{1,2,3}` at `s==1`, backward
  `{N,N-1,N-2}` at `s==N`, central `{s-1,s,s+1}` otherwise — the
  identical `needed`-set construction `crates/mili-py` uses), pass
  `db.times()` and `requested_state_nums == [s]`, call
  `compute_node_velocity` / `compute_node_acceleration`.

The resulting node-label-keyed map feeds a **factored** node-direct
helper extracted *verbatim* from M3's existing inline nodal branch
(`geometry.rs:418-430`) — the M3 primal nodal path calls the same
helper, so its encoded bytes (and every prior gating test) are
unchanged. On any unresolved spec / absent `node` class / failed
`query_full` / failed `compute_*` the branch returns `None` and the
caller falls back to the M3 bare hull. No proto change, no
blob-format change, no `parity` feature.

**Trade-off recorded.** Generalizing the M5/M5b element seam to also
cover the nodal families was rejected: they are a *different gather*
(node-direct, multi-state window, state-0 reference) with no element
class and no IP axis — folding them into the element scatter would
couple two unrelated shapes, exactly the anti-pattern Decision 23
warned against. Extracting M3's nodal mapping into a shared helper
(rather than duplicating it) keeps the M3 path byte-identical *and*
DRY; the helper is a pure move, reviewable as a no-op for the primal
path.

### Decision 30 — `surfstrain*` routing: a separate, clearly-reviewable per-face Hex connectivity gather (`scatter_hex_faces`), nodal-averaging the parity-exact `Database::surface_strain_query` per-element face value onto that face's 4 corner nodes via a viz-local canonical hex face table; the M5/M5b element scatter is untouched

`surfstrain*` is the genuinely new routing shape this slice's design
note flagged. `mili_rs::Database::surface_strain_query(mesh, class,
req_labels, state_idx, face, jr, ic, name, title)` is parity-exact
(`crates/mili-py` drives it, `database.rs:1619-1662`) but computes one
**face** (1–6) at a time, returning a per-element value for that face.
griz colors surface strain on Hex faces; the viz analogue of M3's
"element value → its nodes, nodal-averaged" is "**face** value → that
**face's** 4 nodes, nodal-averaged".

**Decision: add a dedicated `MeshTopology::scatter_hex_faces` method
and a `surfstrain_spec` branch in `vertex_scalar` (after the
nodal-time group, before the primal lookup). For each retained element
class whose superclass is `Superclass::Hex` (resolved via
`db.superclass_code` + `Superclass::from_code`), call
`surface_strain_query(mesh, class, None, &[state_idx], face, jr, ic,
name, title)` for `face ∈ 1..=6`; for each Hex element scatter that
face's per-element value onto the 4 global nodes of that face
(`conns[e*8 + HEX_FACE_NODES[face-1][m]]`), accumulating a per-node
mean exactly as `scatter_elements` does for the element seam.**
`HEX_FACE_NODES` is a viz-local `[[usize;4];6]` constant transcribed
from `reference/mili-python/src/mili/miliinternal.py:675-682` — the
**same** table `surface_strain_query` indexes internally with `face`,
so the face number and the scattered nodes correspond. This is a
**connectivity constant**, the same category as the existing
`triangulation()` table (griz `faces.c`) the viz crate already
mirrors — **not** a derived-formula re-port (the strain math stays
solely in the parity-exact kernel; Decision 19's boundary is about
formulas, not topology tables). The path is **separate** from
`scatter_elements` (Decision 22's explicit rationale — do not contort
the per-face gather into the element-class seam); the M5/M5b element
scatter and the M3 paths are not touched and stay byte-stable. On a
non-Hex-only corpus / absent class / failed `surface_strain_query` the
branch returns `None` → M3 bare hull. No proto change, no
blob-format change, no `parity` feature.

**Trade-off recorded.** Exporting `mili-rs::FACE_TO_NODES` instead of
a viz-local copy would eliminate the (small) drift risk of a duplicated
constant. Rejected for this slice: it is a change to the **core**
crate's public surface for the benefit of a producer slice, the table
is a frozen canonical constant (`miliinternal.py:675-682`, already
mirrored once for `triangulation()` without incident), and a cited
6×4 transcription is reviewable at a glance. A face-provenance field
on every hull triangle (so surfstrain could ride the existing triangle
buffer) was also rejected: it would perturb the M2 hull builder and
risk the byte-stable `MVG1`/`MVG2` blob for zero functional gain — the
element-class `conns` already retained for the M3 scatter carry all
the connectivity `scatter_hex_faces` needs.

### Decision 31 — the gating test validates routing via single-shared-gather invariants only (the exact displacement-magnitude norm identity; structural + state-tracking for `surfstrain*` and the kinematic families; the `vel_*`-at-state-1-is-zero kernel fact) — no cross-cardinality checks; the kernels are not re-validated

Per Decision 24 the test asserts only invariants whose every term
rides one and the same primal gather. For the nodal families one such
identity is exact and skew-free: **`disp_mag` is the Euclidean norm of
`disp_x/y/z`** (and `disp_rad_mag_xy` of `disp_x/y`) — all read the
same single-state `ux/uy/uz` primal with the same state-0 reference on
the IP-free `node` class, and the M3 nodal mapping is node-*direct*
(no average), so the served `disp_mag` equals
`sqrt(disp_x² + disp_y² + disp_z²)` per vertex, exact to f32. This is
the nodal-family analogue of M5's linear-pressure identity: it pins
that the routing wires the right primal into the right kernel into the
right node-direct mapping. `surfstrain*` has no simple cross-component
algebraic identity (a full tensor rotation); like `vol_strain` in M5b
it gets structural + state-tracking coverage — its numeric correctness
is owned by the `mili-rs` core parity suite (Decision 19), not
re-pinned here.

**Decision: `crates/mili-viz-server/tests/m5c_derived.rs`
`derived_surfstrain_and_nodal_time` asserts, at a stressed state on
the transient `serial/basic1` corpus: (a) the norm identities
`disp_mag ≈ ‖(disp_x,disp_y,disp_z)‖` and
`disp_rad_mag_xy ≈ ‖(disp_x,disp_y)‖` per vertex within an f32
tolerance over all finite nodes, with a non-trivial non-zero sample;
(b) every family in scope (`disp_*`, `disp_mag`, `disp_rad_mag_xy`,
`vel_*`, `acc_*`, `surfstrain{x,y,z,xy,yz,zx}`) yields the `MVG2`
layout, a per-vertex scalar of `num_vertices` length with finite
samples, and `ResultState.{min,max}` bracketing the finite values;
(c) `vel_x` at state 1 is identically zero (a kernel-defined
same-gather fact, `derived.py:1062`) and a nodal family scalar tracks
the state (differs between two states); (d) an unknown derived name,
the empty result, **and a re-deferred `*_alt` name
(`prin_strain1_alt`)** each fall back to the M3 bare hull (`MVG1`, no
scalar) — no error; (e) all six M1 + `m2_geometry.rs` +
`m3_primal.rs` + `m4_visibility.rs` + `m5_derived.rs` +
`m5b_principal.rs` + `m6_transport.rs` tests still pass unchanged
(M5/M5b element scatter + M3 primal/nodal path byte-stable; no
`parity` feature added to `mili-viz-server`). Skip-on-absent per
CLAUDE.md (early `return` + `eprintln!` when `serial/basic1` is
absent).** Cross-cardinality "trace"-style phrasings are not used (per
Decision 24 — the IP-sampling skew on the IP-inconsistent corpus is
real and expected).

**Trade-off recorded.** A literal `mili`-oracle comparison (add
`parity` to `mili-viz-server`, diff via pyo3) was rejected, identical
reasoning to M5 Decision 21 / M5b Decision 24: it breaches the M2
`mili-viz-server`-depends-on-`mili-rs`-only boundary for coverage the
core suite already owns. The norm identity already proves the
nodal-time routing end to end; structural + state-tracking is the
right depth for `surfstrain*` (its tensor numerics are core-parity
owned). The residual cost — `surfstrain*`'s absolute values are not
checked in this test — is exactly the intended division of labor.

## M5c acceptance gate

- [x] `show disp_x`/`disp_y`/`disp_z`/`disp_mag`/`disp_rad_mag_xy`
      after `load` each yield `layout == "MVG2:..."`, a fetchable
      per-vertex scalar of `num_vertices` length, and the norm
      identities `disp_mag ≈ ‖(disp_x,disp_y,disp_z)‖`,
      `disp_rad_mag_xy ≈ ‖(disp_x,disp_y)‖` hold per node within an
      f32 tolerance (same gather, exact).
- [x] `show vel_x`/`vel_y`/`vel_z`/`acc_x`/`acc_y`/`acc_z` each yield
      `MVG2`, finite samples, and `ResultState.{min,max}` bracketing
      the finite values; `vel_x` at state 1 is identically zero.
- [x] `show surfstrainx`/`y`/`z`/`xy`/`yz`/`zx` each yield `MVG2`,
      finite samples on Hex nodes, and `ResultState.{min,max}`
      bracketing the finite values.
- [x] A nodal family scalar tracks the state (differs between two
      states on the transient corpus).
- [x] An unknown/unsupported derived name, the empty result, and a
      re-deferred `*_alt` name fall back to the M3 bare hull (`MVG1`,
      no scalar) — no error.
- [x] All six M1 acceptance tests + `m2_geometry.rs` + `m3_primal.rs`
      + `m4_visibility.rs` + `m5_derived.rs` + `m5b_principal.rs` +
      `m6_transport.rs` still pass unchanged (M5/M5b element scatter +
      M3 primal/nodal path byte-stable; no `parity` feature added to
      `mili-viz-server`).
- [x] New test follows the CLAUDE.md skip-on-absent discipline (early
      `return` + `eprintln!` when the corpus fixture is absent).
      → `crates/mili-viz-server/tests/m5c_derived.rs`
      `derived_surfstrain_and_nodal_time`
- [x] `status.md` updated (TL;DR, a Phase 4 sub-bullet, "what is
      decided" table with a `phase-4-m5c.md` row, "immediate next
      steps"); `README.md` open-questions table unaffected (no proto
      change; Q8 closed by M5 Decision 19).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 28 | Family set = nodal time-derived + `surfstrain{x,y,z,xy,yz,zx}`; `*_alt` re-deferred (no parity-exact `mili-rs` kernel — routing it would breach M5 Decision 19; belongs in a future `mili-rs` core derived sub-slice); corrects Decision 22's "nodal-time already M3-reachable" imprecision | M5c scope |
| 29 | Nodal-time routing: node-direct branch group mirroring the `crates/mili-py` nodal dispatch for the single current state; M3's node→vertex mapping factored into a shared helper (M3 primal nodal path byte-stable); element scatter untouched | M5c nodal routing |
| 30 | `surfstrain*` routing: separate `scatter_hex_faces` per-face Hex gather over the parity-exact `Database::surface_strain_query`, nodal-averaged via a viz-local canonical hex face table (`miliinternal.py:675-682`, a connectivity constant, not a formula re-port); M5/M5b element scatter + M3 paths byte-stable | M5c surfstrain routing |
| 31 | Gating test uses only single-shared-gather invariants (the exact displacement-magnitude norm identity; structural + state-tracking for `surfstrain*`/kinematics; `vel_*`-at-state-1-zero) + the totality/fallback checks; kernels not re-validated (core parity owns them); no `parity` feature in `mili-viz-server` | M5c test |
