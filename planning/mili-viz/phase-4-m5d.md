# `mili-viz` Phase 4 M5d — the `*_alt` griz closed-form trig principal-strain variants (buildable scope)

> Scope doc for the **fourth M5-family sub-slice**, continuing
> [`phase-4-m5c.md`](phase-4-m5c.md). M5 routed the scalar stress
> invariants; M5b the eigenvalue families; M5c the `surfstrain*` +
> nodal-time families. This slice closes the **one** family M5c
> Decision 28 explicitly re-deferred: the `*_alt` griz closed-form
> trig principal-strain variants (`prin_strain[1-3]_alt` /
> `prin_dev_strain[1-3]_alt`).
>
> It is a **two-part** slice. Part A is a `mili-rs` **core** derived
> sub-slice (the analogue of the Phase-H derived sub-slices): a
> parity-validated kernel — because, per M5c Decision 28, **no
> parity-exact `mili-rs` kernel existed for `*_alt`**, and M5
> Decision 19's load-bearing boundary forbids the viz server from
> re-porting a formula or enabling the `parity` feature. Part B is the
> trivial viz routing follow-up M5c Decision 28 promised: once the
> kernel exists, the seam is the **identical** M5b element
> nodal-average scatter, only the `*_spec`/`*_primals`/`compute_*`
> calls swapped.
>
> Read [`status.md`](status.md) first, then `phase-4-m5.md`
> Decisions 19–21, `phase-4-m5b.md` Decisions 22–24, and
> `phase-4-m5c.md` Decisions 28–31 — the derived validation philosophy
> this doc continues **verbatim**: the `mili-rs`/`mili-py` core parity
> suite owns kernel numerics; the viz gating test owns only the
> *routing* and rides a **single shared gather** only; no formula
> re-port, no griz golden, **no `parity` feature in
> `mili-viz-server`**. Decisions continue the log (M1: 1–9; M2: 10–12;
> M3: 13–15; M4: 16–18; M5: 19–21; M5b: 22–24; M6: 25–27; M5c: 28–31;
> this slice starts at **32**).

## Goal

`show <name>` after a `load`, for
`prin_strain1_alt` / `prin_strain2_alt` / `prin_strain3_alt` /
`prin_dev_strain1_alt` / `prin_dev_strain2_alt` /
`prin_dev_strain3_alt`, routes through the now-parity-gated
`mili_rs::compute_principal_strain_alt` kernel into the **unchanged**
M3/M5b element nodal-average scatter, keeping the `MVG1`/`MVG2` blob
format, `flight_ticket`, and `ResultState.{min,max}` autoscale
byte-stable for every prior path. Unknown / unresolvable names,
class-absent, or query-failure still fall back to the M3 bare hull
("`show` is total" — M3 Decision 13 / M5 Decision 20).

Out of scope: nothing remains in the M5 derived family — this closes
it. Phase 5 (the client) is the next milestone.

## Part A summary (the `mili-rs` core sub-slice — recorded in detail in `../mili-py/m4.md` Decision 27)

`mili_rs::compute_principal_strain_alt` +
`principal_strain_alt_spec` / `_primals` + the `PrincipalStrainAlt`
enum were added to `crates/mili-rs/src/derived.rs` (re-exported from
`lib.rs`), wired into `crates/mili-py/src/database.rs` (the `query()`
derived dispatch arm + the per-fragment guard), and gated bit-faithful
vs the `mili` Python package by
`crates/mili-py/tests/test_alt_strain_parity.py`. The full decision —
including why the gate is `np.allclose` to a tight f32 tolerance rather
than bitwise `np.array_equal` (numpy's float32 `arccos`/`cos` are
numpy's own SIMD single-precision polynomials, ≠ system libm, not
cross-language bit-reproducible; worst observed abs deviation ≈ 1.7e-10
on d3samp6 vs strain magnitudes ~1e-2; upstream mili-python itself
ships **no** `*_alt` value test, and the `*_alt` docstrings call them
debug-only "alternate calculation methods … to … check[] to see if the
methods matched") — is `../mili-py/m4.md` **Decision 27**. The
`mili-rs`/`mili-py` strict 0-xfail harness stays green
(`pytest -q crates/mili-py/tests`: 950 passed).

## Decisions (continuing the log)

### Decision 32 — `*_alt` is its own enum/spec (`PrincipalStrainAlt`), not an extension of `PrincipalStrain`; the viz seam is byte-for-byte the M5b element-scatter branch with only the `*_spec`/`*_primals`/`compute_*` calls swapped

`*_alt` is a *distinct algorithm* from the non-alt principal strains:
the non-alt family is a symmetric-3×3 Jacobi eigensolver (f64,
[`PrincipalStrain`]); `*_alt` is a closed-form `J2`/`J3` load-angle
solve with f32 transcendentals. Upstream registers them as **separate**
`compute_function`s with `supports_batching=False`
(`derived.py:219-290`).

**Decision: mirror upstream's dispatch structure** — a separate
`PrincipalStrainAlt` enum + `principal_strain_alt_spec` /
`principal_strain_alt_primals` / `compute_principal_strain_alt` (all
six names read all six strain components), **not** new variants on
`PrincipalStrain`. The viz routing is then a verbatim copy of the M5b
`principal_strain_spec` branch in `MeshTopology::vertex_scalar`,
inserted immediately after it, with only `principal_strain_alt_spec` /
`principal_strain_alt_primals` / `compute_principal_strain_alt`
substituted: `classes_of_state_variable("ex")` → `scatter_elements`
(the unchanged M3/M5b element nodal-average) → `component0_map`. On any
unresolved spec / absent class / failed `query_full` / failed
`compute_*` the branch returns `None` → M3 bare hull.

**Trade-off recorded.** Folding `*_alt` into `PrincipalStrain`
(reusing one branch) was rejected: it would couple two unrelated
numeric kernels (eigensolver vs closed-form trig) behind one enum,
obscure the very different parity contracts (bitwise vs f32-tolerance),
and diverge from upstream's own separate-`compute_function` structure —
the M5/M5b boundary discipline (Decision 23: do not contort unlike
shapes into one seam) applied to the kernel layer. Cost: ~30 lines of
deliberately parallel routing — reviewable at a glance against the
non-alt branch, and isolating the tolerance-gated family.

### Decision 33 — gating test = single-shared-gather invariants only: structural + `ResultState` bracketing, the **principal-ordering** identity `1_alt ≥ 2_alt ≥ 3_alt` per vertex, state-tracking, and the totality/Decision-28-closure check; the kernel is **not** re-validated here

Per `phase-4-m5b.md` Decision 24 / `phase-4-m5c.md` Decision 31 the
viz test asserts only invariants whose every term rides one and the
same primal gather; kernel numerics are owned by the core parity suite
(Part A / `../mili-py/m4.md` Decision 27), not re-pinned here. For
`*_alt` one identity is exact and skew-free under the shared gather:
**`prin_strain1_alt ≥ prin_strain2_alt ≥ prin_strain3_alt`** per vertex
(and likewise the `dev` family). It holds *per element* by the
load-angle construction — `θ₁ = arccos(α)/3 ∈ [0,π/3]`,
`θ₂ = θ₁−2π/3`, `θ₃ = θ₁+2π/3`, with `value ≥ 0` and a common
`+e_hyd`, so `cos θ₁ ≥ cos θ₂ ≥ cos θ₃` ⇒ component 1 ≥ 2 ≥ 3 (limit-
fail elements are all 0, so `≥` still holds) — and the M5b nodal
average is monotone over the **same** weights/elements for all three
components (one shared scatter), so the ordering survives per vertex.
This is the M5d analogue of M5c's displacement-norm identity.

**Decision: `crates/mili-viz-server/tests/m5d_alt_strain.rs`
`derived_alt_principal_strain` asserts, at a deformed state (22) on the
transient `serial/sstate/d3samp6` corpus: (a) every one of the six
`*_alt` names yields `MVG2`, a per-vertex scalar of `num_vertices`
length with finite samples, and `ResultState.{min,max}` bracketing the
finite values; (b) the per-vertex ordering `1_alt ≥ 2_alt ≥ 3_alt`
(and the `dev` triple) within an f32 relative slack (the same shape as
M5c's norm tolerance — f32 averaging noise only); (c) a non-vacuous
check (`prin_strain1_alt` has a finite non-zero vertex — the limit mask
did not zero everything); (d) state-tracking (a `*_alt` scalar differs
between state 22 and 60); (e) an unknown name and the empty result
fall back to the M3 bare hull (`MVG1`, no scalar) — no error — while
`prin_strain1_alt` now resolves to `MVG2` (the explicit closure of
Decision 28); (f) all prior gating tests still pass unchanged.
Skip-on-absent per CLAUDE.md (early `return` + `eprintln!`).**
Cross-cardinality "trace"-style checks are not used (Decision 24 — the
IP-sampling skew on the IP-inconsistent corpus is real and expected;
and `*_alt`'s absolute numerics are core-parity owned).

**Trade-off recorded.** A literal `mili`-oracle comparison in the viz
test (add `parity` to `mili-viz-server`, diff via pyo3) was rejected,
identical reasoning to M5 Decision 21 / M5b Decision 24 / M5c
Decision 31: it breaches the M2
`mili-viz-server`-depends-on-`mili-rs`-only boundary for coverage Part
A's `crates/mili-py` tolerance gate already owns. Corpus choice:
`serial/sstate/d3samp6` (not M5c's `serial/basic1`) — it is upstream's
canonical strain corpus (`SerialDerivedExpressions`), transient (101
states → state-tracking is real), and the `*_alt` family resolves on
its `brick` class; `basic1`'s only distinguishing value is its
IP-inconsistency, which is irrelevant to an element-scatter ordering
invariant and which Decision 24 forbids leaning on anyway.

### Decision 34 — `phase-4-m5c.md` Decision 28 is now discharged; the M5c gating test's `*_alt`→bare-hull assertion is intentionally *removed* (superseded, not regressed) and relocated to M5d

`m5c_derived.rs` `derived_surfstrain_and_nodal_time` previously
asserted `prin_strain1_alt` / `prin_dev_strain2_alt` fall back to the
M3 bare hull — the runtime expression of M5c Decision 28's
re-deferral. With Part A's kernel + this slice's seam, those names now
*resolve* to an `MVG2` scalar, so that assertion is **deliberately
deleted** from `m5c_derived.rs` (the two `*_alt` names dropped from its
totality-fallback list; `not_a_derived` / `""` retained) and the
positive-resolution + Decision-28-closure coverage moved to
`m5d_alt_strain.rs`.

**Decision: treat this as a *superseded* assertion, not a regression.**
`phase-4-m5c.md` Decision 28 stays in the historical log unedited
(decisions are append-only); this Decision 34 records that it is
discharged and *where* its runtime check moved. The M5c test otherwise
stays byte-stable (surfstrain + nodal-time coverage untouched, still
green on `serial/basic1`).

**Trade-off recorded.** Leaving the `*_alt`→bare-hull assertion in
`m5c_derived.rs` (and not routing `*_alt`) would keep the doc/test
literally matching Decision 28, but that *is* the deferral this slice
exists to close, and a now-false "must be bare hull" assertion would
hard-fail CI. Editing the assertion in place (rather than deleting it)
was rejected: M5d, not M5c, owns `*_alt` routing coverage; M5c's job is
surfstrain/nodal-time, and its totality check is about *unknown* names,
which `*_alt` no longer is.

## M5d acceptance gate

- [x] **Part A**: `compute_principal_strain_alt` +
      `principal_strain_alt_spec`/`_primals` + `PrincipalStrainAlt` in
      `crates/mili-rs/src/derived.rs`, re-exported from `lib.rs`, wired
      into `crates/mili-py/src/database.rs` (dispatch arm +
      per-fragment guard), bit-faithful vs the `mili` Python package via
      `crates/mili-py/tests/test_alt_strain_parity.py` (np.allclose,
      rtol 1e-5 / atol 1e-6 — m4.md Decision 27); strict 0-xfail
      harness green (`pytest -q crates/mili-py/tests`: 950 passed).
- [x] `show prin_strain{1,2,3}_alt` / `prin_dev_strain{1,2,3}_alt`
      after `load` each yield `MVG2`, a fetchable per-vertex scalar of
      `num_vertices` length, finite samples, `ResultState.{min,max}`
      bracketing, and the per-vertex ordering `1_alt ≥ 2_alt ≥ 3_alt`
      (and the `dev` triple) within an f32 slack.
- [x] A `*_alt` scalar tracks the state (differs between two states on
      the transient corpus).
- [x] An unknown/unsupported name and the empty result fall back to
      the M3 bare hull (`MVG1`, no scalar) — no error; `prin_strain1_alt`
      now resolves to `MVG2` (Decision 28 discharged).
- [x] All six M1 acceptance tests + `m2_geometry.rs` + `m3_primal.rs`
      + `m4_visibility.rs` + `m5_derived.rs` + `m5b_principal.rs` +
      `m5c_derived.rs` (its `*_alt`→bare-hull assertion intentionally
      removed per Decision 34) + `m6_transport.rs` still pass (M5/M5b
      element scatter + M3 path byte-stable; no `parity` feature added
      to `mili-viz-server`).
- [x] No proto change; `MVG1`/`MVG2` blob, `flight_ticket`, autoscale
      byte-stable; `README.md` open-questions table unaffected (Q8
      closed by M5 Decision 19).
- [x] `cargo fmt --all --check` + `cargo clippy --workspace
      --all-targets -- -D warnings` clean; full non-parity
      `cargo test --workspace` green.
- [x] New test follows the CLAUDE.md skip-on-absent discipline (early
      `return` + `eprintln!` when the fixture is absent).
      → `crates/mili-viz-server/tests/m5d_alt_strain.rs`
      `derived_alt_principal_strain`
- [x] `status.md` updated (TL;DR, Phase 4 sub-bullet, "what is decided"
      table with a `phase-4-m5d.md` row, "immediate next steps");
      `../mili-rs/status.md` updated (new public symbols + the numpy
      f32-transcendental surprise); `../mili-py/m4.md` Decision 27
      written.

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 32 | `*_alt` is its own `PrincipalStrainAlt` enum/spec (mirrors upstream's separate `compute_function`s), not an extension of `PrincipalStrain`; the viz seam is the verbatim M5b element-scatter branch with only `*_spec`/`*_primals`/`compute_*` swapped | M5d kernel+routing structure |
| 33 | Gating test = single-shared-gather invariants only (structural + `ResultState` bracketing; the per-vertex principal-ordering `1≥2≥3` identity; state-tracking; totality + Decision-28-closure); kernel numerics core-parity owned (m4.md Decision 27), not re-validated; corpus `serial/sstate/d3samp6` | M5d test |
| 34 | `phase-4-m5c.md` Decision 28 discharged; the M5c `*_alt`→bare-hull assertion intentionally removed (superseded, not regressed) and its positive-resolution coverage relocated to `m5d_alt_strain.rs`; Decision 28 stays append-only in the historical log | closes M5c Decision 28 |
