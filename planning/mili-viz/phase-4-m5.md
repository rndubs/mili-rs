# `mili-viz` Phase 4 M5 — derived results (buildable scope)

> Scope doc for Phase 4 Milestone 5, continuing
> [`phase-4-m4.md`](phase-4-m4.md). M3 made `show <primal>` carry a
> per-vertex scalar; M4 added material filtering. M5 makes
> `show <derived>` work for the **scalar stress invariants** by routing
> through the already-parity-exact `mili-rs` derived kernel, then
> reusing M3's nodal scatter. No proto change — the scalar rides the
> same `MVG2` blob; this is server-side routing only.
>
> Read [`status.md`](status.md) first, then `phase-4-m4.md` and
> `phase-4-m1.md` Decision 5 (the M5 validation pre-commitment).
> Reference behavior is read-only griz under `reference/griz/Src/`;
> the formula source of truth is `reference/mili-python/src/mili/
> derived.py`, already transcribed bit-exact into
> `crates/mili-rs/src/derived.rs`. Decisions continue the log
> (M1: 1–9; M2: 10–12; M3: 13–15; M4: 16–18; M5 starts at 19).

## Goal

`show <stress-invariant>` (e.g. `pressure`, `eff_stress`,
`triaxiality`, `norm_press`) after a `load`:

- resolves the name to the `mili-rs` stress-invariant kernel,
- queries the component stress primals on each prepped element class at
  the current state and computes the invariant per element,
- maps the per-element values onto the mesh as a per-vertex scalar via
  **M3's exact nodal-average path** (same `MVG2` blob, same
  `ResultState.{min,max}` autoscale),

so a Phase-5 client colors a derived field exactly as it colors a
primal one. Unknown / unresolvable names still fall back to the M3
bare hull (the "`show` is total" invariant from M3 Decision 13 holds).

Out of scope (deferred sub-slices, see Decision 20): the
eigenvalue-based families (`prin_stress*`, `prin_dev_stress*`,
`max_shear_stress`, `prin_strain*`, `vol_strain`), `surfstrain*`
(per-face Hex), and the nodal time-derived families
(velocity/acceleration/displacement already reachable as primals in
the M3 path). Flight-over-TCP is still M6.

## Decisions (continuing the log)

### Decision 19 — the derived oracle exists after all (`mili-rs::derived`, bit-exact vs the `mili` Python package via the frozen Phase-1–3 parity suite); M5-viz reuses it and adds **no** formula port and **no** griz golden — superseding `phase-4-m1.md` Decision 5's fallback

`phase-4-m1.md` Decision 5 pre-committed a validation strategy *under
the assumption* that "there is no upstream oracle for viz derived
results", landing on a committed griz golden + tolerance with the
formulas re-transcribed into the M5 doc. That assumption is **false in
this repo**: Phases 1–3 already ported every stress/strain derived
expression from `reference/mili-python/src/mili/derived.py` into
`crates/mili-rs/src/derived.rs` (`compute_stress_invariant` and
friends, exported from the crate root), and that port is **bit-exact
validated against the `mili` Python package** by the frozen
`mili-rs`/`milox` parity suite (`scripts/setup-parity.sh`, the
`parity` feature).

**Decision: M5-viz computes derived results by calling the existing
public `mili_rs::{stress_invariant_spec, stress_invariant_primals,
compute_stress_invariant}` — the same kernel `crates/mili-py` drives
(`crates/mili-py/src/database.rs` `query()`, the stress-invariant
arm ~1493–1526). The formula is therefore neither re-ported into the
viz crate nor re-validated against griz; the M5 gating test validates
the *viz routing* (derived dispatch + the M3 nodal scatter), not the
kernel. No griz binary, no committed golden, and — preserving the M2
"`mili-viz-server` depends on `mili-rs`, no `pyo3`/`parity`" boundary —
no `parity` feature is added to `mili-viz-server`.** This explicitly
supersedes `phase-4-m1.md` Decision 5: the griz-golden path was a
fallback for "no oracle"; the oracle exists, is in-repo, CI-runnable,
and bit-exact, so the fallback is unnecessary.

**Trade-off recorded.** Honoring Decision 5 literally (commit a griz
golden, re-transcribe formulas into this doc) would be faithful to the
pre-commitment but would (a) duplicate the `derived.py`→`derived.rs`
transcription a third time for one milestone, (b) reintroduce the
heavy/flaky griz dependency Decision 5 itself rejected, and (c) drift
from the single kernel `mili-py` already ships. Rejected. The cost of
the chosen path — a forward reference from Decision 5 to here — is
recorded in `status.md` so a cold reader sees the supersession.

### Decision 20 — M5's first (this PR's) slice is the **scalar** stress invariants; the eigensolver / per-face / time-derived families are explicitly deferred sub-slices

`README.md` Phase 4 M5 reads "Port stress invariants, **then** strain";
`crates/mili-rs/src/derived.rs` itself comments the principal-stress /
principal-strain family as "a later sub-slice", and `crates/mili-py`
landed derived as ordered sub-slices, not one drop.

**Decision: this PR implements the four scalar stress invariants
`pressure` / `eff_stress` / `triaxiality` / `norm_press`
(`mili_rs::stress_invariant_spec`). `show <name>` resolves the name,
resolves the carrying element classes via
`Database::classes_of_state_variable(<first component primal>)` (the
invariant lives wherever `sx` lives), queries the component primals on
each class with `Database::query_full` at the current state, calls
`mili_rs::compute_stress_invariant`, and feeds the resulting
per-element `label → value` map into M3's **unchanged** element
nodal-average scatter. The `MVG2` blob, the `NaN`-for-untouched
convention, the component-0 rule, and the `ResultState.{min,max}`
autoscale are exactly M3 Decisions 14–15 — derived is just a different
producer of the per-element map. A derived name that does not resolve,
whose class is absent, or whose primal query fails falls back to the
M3 bare hull (M3 Decision 13's "`show` is total" invariant — `show`
never errors).** The eigenvalue families (`prin_stress*`,
`prin_dev_stress*`, `max_shear_stress`, `prin_strain*`, `vol_strain`),
`surfstrain*` (per-face Hex connectivity), and the contact/eps-rate/
cog families are out of this slice — they are reachable through the
identical routing in a follow-up sub-slice (the kernels already exist
in `mili-rs`), and gating them here would balloon one milestone.

**Trade-off recorded.** Shipping every derived family at once would
"finish derived" in one PR. Rejected: it contradicts the README's
explicit "invariants, then strain" ordering and the established
per-milestone-PR sub-slice discipline (`mili-py` precedent), and the
eigensolver families need extra per-element shape assembly that
deserves its own reviewable slice. The cost — a follow-up PR for the
remaining families — is bounded because the routing built here is the
reusable seam (resolve → per-class `query_full` → `compute_*` → M3
scatter); the follow-up only swaps the `compute_*` call.

### Decision 21 — the gating test validates the routing via the **linear-pressure identity** (nodal-average commutes with the linear combination), with structural assertions for the nonlinear invariants

The nodal scatter (M3) is a per-node arithmetic mean; `pressure` is the
**linear** map `-1/3·(sx+sy+sz)`. Averaging commutes with a linear
combination, so the M5-served per-vertex `pressure` must equal
`-1/3·(P_sx + P_sy + P_sz)` node-by-node, where `P_*` is the M3-served
per-vertex scalar for that primal — an exact (to f32 round-off)
cross-check that exercises the real `compute_stress_invariant` kernel
*through the viz path* and needs no connectivity, no external oracle,
and no committed fixture.

**Decision: `crates/mili-viz-server/tests/m5_derived.rs`
`derived_stress_invariants` asserts (a) the linear identity above for
`pressure` within an f32 tolerance over all finite nodes; (b)
`eff_stress` / `triaxiality` / `norm_press` produce the `MVG2` layout,
a per-vertex scalar of `num_vertices` length with finite samples on
resulted elements, and `ResultState.{min,max}` bracketing the finite
values; (c) the derived scalar tracks the state on the transient
corpus; (d) an unknown derived name falls back to the M3 bare hull;
(e) all six M1 + M2 + M3 + M4 tests still pass unchanged. Skip-on-
absent per CLAUDE.md (`serial/basic1`).** The nonlinear invariants get
structural — not value — assertions here because their kernels are
*already* bit-exact-validated against the `mili` Python package in the
`mili-rs` core parity suite (Decision 19); re-pinning their numeric
output in the viz test would duplicate that and couple a viz test to
the corpus's exact values for zero added coverage of the viz routing.

**Trade-off recorded.** A literal `mili`-oracle comparison (add the
`parity` feature to `mili-viz-server`, recompute via pyo3, diff) would
re-validate the kernel end-to-end through viz. Rejected: it breaches
the M2 `mili-viz-server`-depends-on-`mili-rs`-only boundary for
coverage the core suite already owns; the linear identity already
proves the viz routing wires the right primals into the right kernel
and the right nodal scatter, which is M5-viz's actual contract.

## M5 acceptance gate

- [x] `show pressure` after `load` yields `layout == "MVG2:..."`, a
      fetchable per-vertex scalar of `num_vertices` length, and the
      per-node linear identity `pressure ≈ -1/3·(sx+sy+sz)` holds
      within an f32 tolerance against the M3-served primals.
- [x] `show eff_stress` / `triaxiality` / `norm_press` each yield
      `MVG2`, finite samples on resulted elements, and
      `ResultState.{min,max}` bracketing the finite values.
- [x] The derived scalar tracks the state (differs between two states
      on the transient corpus).
- [x] An unknown/unsupported derived name falls back to the M3 bare
      hull (`MVG1`, no scalar) — no error.
- [x] All six M1 acceptance tests + `m2_geometry.rs` + `m3_primal.rs`
      + `m4_visibility.rs` still pass unchanged (primal path
      byte-stable; no `parity` feature added to `mili-viz-server`).
- [x] New test follows the CLAUDE.md skip-on-absent discipline (early
      `return` + `eprintln!` when the corpus fixture is absent).
      → `crates/mili-viz-server/tests/m5_derived.rs`
      `derived_stress_invariants`
- [x] `status.md` M5 box flipped with the gating test named, and the
      Decision-5 supersession noted; `README.md` open-questions table
      unaffected (no proto change; Q8 closed by Decision 19).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 19 | Derived oracle exists (`mili-rs::derived`, bit-exact vs `mili` Python); reuse it, no formula port, no griz golden; supersedes `phase-4-m1.md` Decision 5 | M5 validation strategy |
| 20 | First slice = scalar stress invariants via resolve → per-class `query_full` → `compute_stress_invariant` → M3 nodal scatter; eigensolver / per-face / time families deferred; unresolvable → bare hull | M5 scope |
| 21 | Gating test uses the linear-pressure identity (avg commutes with the linear combo) + structural assertions for the nonlinear invariants; no `parity` feature in `mili-viz-server` | M5 test |
</content>
