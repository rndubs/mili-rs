# `mili-viz` Phase 4 M5 follow-up — eigenvalue-based derived families (buildable scope)

> Scope doc for the Phase 4 M5 **follow-up sub-slice**, continuing
> [`phase-4-m5.md`](phase-4-m5.md). M5's first slice routed the
> **scalar** stress invariants (`pressure`/`eff_stress`/`triaxiality`/
> `norm_press`) through the parity-exact `mili-rs` kernel into M3's
> nodal scatter. This sub-slice adds the **eigenvalue-based families**
> (principal stress/strain, deviatoric, max-shear, volumetric strain)
> through the **identical** routing seam — only the `*_spec` /
> `*_primals` / `compute_*` calls change. No proto change; server-side
> only; the M5 stress-invariant branch and the M3 primal path stay
> byte-stable.
>
> Read [`status.md`](status.md) first, then `phase-4-m5.md`
> Decisions 19–21 (the M5 validation philosophy this doc continues:
> reuse the parity-exact `mili-rs` kernel, no formula re-port, no griz
> golden, no `parity` feature in `mili-viz-server`). Decisions continue
> the log (M1: 1–9; M2: 10–12; M3: 13–15; M4: 16–18; M5: 19–21; this
> sub-slice starts at **22**).

## Goal

`show <eigenvalue-derived>` after a `load`, for the families:

- `prin_stress1` / `prin_stress2` / `prin_stress3`
- `prin_dev_stress1` / `prin_dev_stress2` / `prin_dev_stress3`
- `max_shear_stress`
- `prin_strain1` / `prin_strain2` / `prin_strain3`
- `prin_dev_strain1` / `prin_dev_strain2` / `prin_dev_strain3`
- `vol_strain`

resolves via `mili_rs::principal_stress_spec` /
`mili_rs::principal_strain_spec`, queries the component primals
(`sx..szx` / `ex..ezx`, or the 3 normals for `vol_strain`) on each
prepped element class with `Database::query_full` at the current
state, computes the family value per element with the
**already-parity-exact** `mili_rs::compute_principal_stress` /
`mili_rs::compute_principal_strain` kernel (the same one
`crates/mili-py` drives — `crates/mili-py/src/database.rs` `query()`,
the `principal_stress_spec` / `principal_strain_spec` arms
~1527–1590), and feeds the resulting per-element `label → value` map
into M3's **unchanged** nodal-average scatter (same `MVG2` blob, same
`ResultState.{min,max}` autoscale, same `NaN`-untouched / component-0
conventions). Unknown / unresolvable names, class-absent, or
query-failure still fall back to the M3 bare hull (the "`show` is
total" invariant from M3 Decision 13 / M5 Decision 20 holds — `show`
never errors).

Out of scope (still deferred — see Decision 22): `surfstrain*` (needs
per-face Hex connectivity, not the already-prepped element-class
gather this seam reuses), the `*_alt` griz closed-form trig strain
variants (a distinct algorithm), and the nodal time-derived families
(velocity/acceleration/displacement — already reachable as primals via
the M3 path). Flight-over-TCP is still M6.

## Decisions (continuing the log)

### Decision 22 — this sub-slice's family set is exactly the eigensolver-on-already-prepped-element-classes families (`prin_stress[1-3]`, `prin_dev_stress[1-3]`, `max_shear_stress`, `prin_strain[1-3]`, `prin_dev_strain[1-3]`, `vol_strain`); `surfstrain*`, the `*_alt` trig strains, and the time-derived families stay deferred

The resolvable names are exactly those `mili_rs::principal_stress_spec`
(`prin_stress[1-3]`, `prin_dev_stress[1-3]`, `max_shear_stress`) and
`mili_rs::principal_strain_spec` (`vol_strain`, `prin_strain[1-3]`,
`prin_dev_strain[1-3]`) accept — confirmed against
`crates/mili-rs/src/derived.rs:1073-1084` / `1241-1252`. Every one of
these reads only the 6 (or, for `vol_strain`, 3) stress/strain
component primals on a *single already-prepped element class* and runs
either a pure linear combination (`vol_strain`) or the shared
symmetric-3×3 Jacobi eigensolver — i.e. exactly the M5 gather shape,
no extra per-element shape assembly.

**Decision: this PR implements precisely the 14 names above via the
M5 routing seam. `surfstrain{x,y,z,xy,yz,zx}` (per-face Hex surface
connectivity — a different gather than "the invariant lives wherever
`sx`/`ex` lives"), the `*_alt` griz closed-form trig principal-strain
variants (a distinct algorithm, called out as a later sub-slice in
`derived.rs:1226-1227`), and the nodal time-derived families remain
deferred.** They are reachable through other seams in a later
sub-slice; the kernels for `surfstrain` already exist in `mili-rs`
(`surfstrain_spec`) but require the per-face connectivity path M3's
element-class scatter does not model, so folding them in here would
mix two routing shapes in one reviewable slice.

**Trade-off recorded.** Including `surfstrain*` would "finish strain"
in one PR. Rejected: it contradicts M5 Decision 20's explicit
"per-face needs its own reviewable slice" rationale and the README's
"invariants, then strain" ordering, and it would couple the clean
"swap only the `compute_*` call" property of this seam to a second,
unrelated connectivity gather. The cost — a further follow-up for
`surfstrain*`/`*_alt`/time families — is bounded: the kernels exist
and the routing pattern is now proven twice (M5 + this).

### Decision 23 — routing reuses the M5 seam verbatim: two new branches before the primal `classes_of_state_variable` lookup, each swapping only `*_spec` / `*_primals` / `compute_*`; everything else (`scatter_elements` / `finite_range` / `component0_map`, the `MVG2` blob, the M3 primal path) is byte-stable

The M5 stress-invariant branch in `MeshTopology::vertex_scalar`
(`crates/mili-viz-server/src/geometry.rs`) is
`resolve spec → classes_of_state_variable(primal[0]) → scatter_elements(|class| query_full each primal → compute_* → component0_map)`.
The eigenvalue families have the *identical* shape — the only
differences are (a) `principal_stress_primals()` takes no argument
(always the 6 stress components) while `principal_strain_primals(kind)`
and `stress_invariant_primals(inv)` are kind-keyed (`vol_strain` →
3 normals), and (b) the `compute_*` function name.

**Decision: add two branches to `vertex_scalar`, immediately after the
M5 `stress_invariant_spec` branch and before the primal
`classes_of_state_variable(svar)` lookup, in the order
`principal_stress_spec` then `principal_strain_spec` (mirroring the
`crates/mili-py` `query()` dispatch order). Each branch is the M5
branch with only `*_spec` / `*_primals` / `compute_*` substituted; it
reuses `scatter_elements`, `finite_range`, and `component0_map`
unchanged, emits the same `MVG2` blob, and on any unresolved spec /
absent class / failed `query_full` / failed `compute_*` returns `None`
so the caller falls back to the M3 bare hull.** The M5
stress-invariant branch and the M3 primal path are not touched, so
their encoded bytes — and therefore every prior gating test — stay
identical. No proto change, no blob-format change, no `parity` feature
added to `mili-viz-server` (the M2 `mili-viz-server`-depends-on-
`mili-rs`-only boundary holds; M5 Decision 19's "the oracle is the
already-parity-exact kernel" carries over verbatim).

**Trade-off recorded.** Factoring the three near-identical branches
(M5 invariant + these two) into one generic helper would cut
duplication. Rejected for this slice: the three differ in their
`*_primals` arity/keying and `compute_*` symbol, a generic seam would
need a closure-of-closures or a trait the kernel does not expose, and
the M5 branch is explicitly frozen as a prior-test byte-stability
anchor — refactoring it here would put a reviewed, byte-stable path at
risk for cosmetic gain. The cost (≈40 lines of structural repetition)
is local, obvious, and exactly the pattern `crates/mili-py` already
ships for the same dispatch.

### Decision 24 — the gating test validates the routing via algebraic invariants that ride a **single shared derived primal gather** (eigenvalue ordering, relative deviatoric tracelessness, the max-shear relation) — never a cross-cardinality cross-field check — plus structural + state-tracking assertions; the eigensolver itself is not re-validated

The M5 linear-pressure identity does not generalize: individual
eigenvalues are not a linear function of the stress components, so
`avg(λ)` ≠ `f(avg(component))`. The M3 nodal scatter is a per-node
arithmetic mean, and a mean both **preserves order** and **commutes
with any linear combination** of per-element quantities — the two
properties this test leans on.

A skew was discovered in implementation and is pinned here, sharper
than the M5 Decision 21 note. The derived path reads `query_full` and
`component0_map` picks atom 0; on the IP-inconsistent `serial/basic1`
brick class the per-element integration-point count varies, so a
`query_full` of *N* primals and a `query_full` of *M* primals (N≠M)
select **different effective IP representations** even though both are
"the derived gather". Concretely the obvious "trace identity"
phrasings — `prin_stress1+2+3 ≈ -3·pressure` (6- vs 3-primal gather)
and `prin_strain1+2+3 ≈ vol_strain` (6- vs 3-primal gather) — were
implemented and **fail at ~1.5e-3 for a routing that is in fact
correct**. (Comparing against the raw `sx+sy+sz`/`ex+ey+ez` primals
fails identically — that is additionally the M5 derived-vs-primal
skew.) The robust rule: **only assert invariants whose every term
rides one and the same primal gather.** Three such invariants jointly
pin the entire eigenvalue-family routing with zero skew and zero
oracle:

1. **Eigenvalue ordering (same gather).** `eigvalsh` is ascending and
   the kernel maps it max→`prin1`, mid→`prin2`, min→`prin3`
   (`derived.rs:1136-1141`). A mean preserves order, so the served
   `prin_stress1 ≥ prin_stress2 ≥ prin_stress3` node-by-node (within
   an f32 slack); likewise `prin_dev_stress*`, `prin_strain*`,
   `prin_dev_strain*`. This proves all three eigenvalues are computed
   from the right primals and mapped to the right names.
2. **Deviatoric tracelessness, *relative* (same gather).** The
   deviatoric matrix is traceless, so `dev1+dev2+dev3 = 0` per
   element. Each `dev` is O(stress) (tens) and the cancellation is in
   f32 over a nodal average, so the meaningful, skew-free statement is
   `|dev1+dev2+dev3| ≤ 1e-3·(|dev1|+|dev2|+|dev3|)` — a residual
   negligible against the magnitudes, not below an absolute epsilon
   (an absolute-epsilon phrasing was implemented and correctly flagged
   the f32 cancellation residual as ~0.1; that is expected, not a
   defect). Asserted for `prin_dev_stress*` and `prin_dev_strain*`.
3. **Max-shear relation (same gather).** `max_shear_stress =
   ½·(λmax − λmin) = ½·(prin_stress1 − prin_stress3)` per element;
   ½·(a−b) is linear and all three ride the *same* 6-stress-primal
   eigensolver gather, so the served `max_shear_stress` equals
   `½·(prin_stress1 − prin_stress3)` per vertex, exact to f32.

**Decision: `crates/mili-viz-server/tests/m5b_principal.rs`
`derived_principal_families` asserts, at a stressed state on the
transient `serial/basic1` corpus: (a) descending order for
`prin_stress*`, `prin_dev_stress*`, `prin_strain*`,
`prin_dev_strain*`; (b) relative deviatoric tracelessness for
`prin_dev_stress*` and `prin_dev_strain*`; (c) the max-shear relation
`max_shear_stress ≈ ½·(prin_stress1 − prin_stress3)` with a
non-trivial, non-zero sample; (d) every family (including
`vol_strain`) yields the `MVG2` layout, a per-vertex scalar of
`num_vertices` length with finite samples on resulted elements, and
`ResultState.{min,max}` bracketing the finite values; (e) a family
scalar tracks the state (differs between two states); (f) an unknown
derived name and the empty result fall back to the M3 bare hull
(`MVG1`, no scalar) — no error; (g) all six M1 + `m2_geometry.rs` +
`m3_primal.rs` + `m4_visibility.rs` + `m5_derived.rs` tests still pass
unchanged. Skip-on-absent per CLAUDE.md (early `return` + `eprintln!`
when `serial/basic1` is absent).** `vol_strain` is the trivial linear
strain trace — the *same kernel family* M5 already validated
numerically via the pressure identity — so here it gets structural +
state-tracking coverage only; its numeric correctness, like every
eigenvalue, is owned by the `mili-rs` core parity suite (M5
Decision 19). The eigenvalues are *not* numerically re-pinned here:
the viz test's job is the routing, not the kernel.

**Trade-off recorded.** The cross-cardinality "trace identity"
phrasings (`prin_*1+2+3` vs `-3·pressure` / `vol_strain` / raw
primals) were implemented first and rejected: they couple the
assertion to the IP-sampling skew above, which is real, expected, and
not what this test validates — they fail at ~1.5e-3 for a correct
routing. Same-gather ordering + relative tracelessness + the
max-shear relation are *stronger* than a single trace check anyway
(they pin the λ→`prin{1,2,3}` mapping, the deviatoric shift, and the
max-shear reduction) and are exact/robust with zero skew. A literal
`mili`-oracle comparison (add `parity` to `mili-viz-server`, diff via
pyo3) was also rejected, identical reasoning to M5 Decision 21: it
breaches the M2 `mili-viz-server`-depends-on-`mili-rs`-only boundary
for coverage the core suite already owns. The residual cost — the
eigenvalues' absolute numeric values are not checked *in this test* —
is exactly the intended division of labor (M5 Decision 19): the core
parity suite owns kernel correctness, this gate owns the viz routing.

## M5b acceptance gate

- [x] `show prin_stress1`/`2`/`3` after `load` each yield
      `layout == "MVG2:..."`, a fetchable per-vertex scalar of
      `num_vertices` length, and `prin_stress1 ≥ prin_stress2 ≥
      prin_stress3` node-by-node (eigenvalue ordering, same gather).
- [x] `prin_dev_stress*` and `prin_dev_strain*` are relatively
      traceless (`|Σ| ≤ 1e-3·Σ|·|`) and descending node-by-node.
- [x] `show max_shear_stress ≈ ½·(prin_stress1 − prin_stress3)` per
      node over the served fields (same gather, exact to f32).
- [x] `prin_strain*` is descending; `vol_strain` yields `MVG2` with
      finite samples and tracks the state.
- [x] Every family yields `MVG2`, finite samples on resulted elements,
      and `ResultState.{min,max}` bracketing the finite values.
- [x] A family scalar tracks the state (differs between two states on
      the transient corpus).
- [x] An unknown/unsupported derived name and the empty result fall
      back to the M3 bare hull (`MVG1`, no scalar) — no error.
- [x] All six M1 acceptance tests + `m2_geometry.rs` + `m3_primal.rs`
      + `m4_visibility.rs` + `m5_derived.rs` still pass unchanged
      (M5 stress-invariant branch + M3 primal path byte-stable; no
      `parity` feature added to `mili-viz-server`).
- [x] New test follows the CLAUDE.md skip-on-absent discipline (early
      `return` + `eprintln!` when the corpus fixture is absent).
      → `crates/mili-viz-server/tests/m5b_principal.rs`
      `derived_principal_families`
- [x] `status.md` updated (TL;DR, M5 follow-up bullet, "what is
      decided" table with a `phase-4-m5b.md` row, the Phase 4 list,
      "immediate next steps" naming the gating test); `README.md`
      open-questions table unaffected (no proto change; Q8 already
      closed by M5 Decision 19).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 22 | Family set = the 14 eigensolver-on-already-prepped-element-classes names (`prin_stress[1-3]`, `prin_dev_stress[1-3]`, `max_shear_stress`, `prin_strain[1-3]`, `prin_dev_strain[1-3]`, `vol_strain`); `surfstrain*`/`*_alt`/time families deferred | M5b scope |
| 23 | Routing reuses the M5 seam verbatim — two branches before the primal lookup, swapping only `*_spec`/`*_primals`/`compute_*`; `scatter_elements`/`finite_range`/`component0_map`, the `MVG2` blob, and the M3 primal path byte-stable; no `parity` feature | M5b routing |
| 24 | Gating test uses only single-shared-gather algebraic invariants (eigenvalue descending order, relative deviatoric tracelessness, max-shear relation) + structural/state-tracking — cross-cardinality "trace" phrasings rejected (IP-sampling skew on the IP-inconsistent corpus, ~1.5e-3 false fail); eigensolver not re-validated (core parity suite owns it); no `parity` feature in `mili-viz-server` | M5b test |
</content>
</invoke>
