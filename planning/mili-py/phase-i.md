# Phase I — parallel per-proc-unmerged surface

> **Status: I.1 + I.2 LANDED; I.3 next.** This is the self-contained
> entry-point doc for the parallel slice. The summary + decision 20
> live in [`m4.md`](m4.md) § "Phase I"; this file carries the
> reproducible starting state so a fresh session can pick it up
> cold. Read [`m4.md`](m4.md) (decisions 16–20) and
> [`../mili-rs/status.md`](../mili-rs/status.md) (the `milox` parity +
> redirect tracker row) first. Architectural precedent: decisions 18
> (verbatim non-parity Python over parity-correct core output) and 19
> (parity-sensitive value/topology math in the Rust core); m1.md
> § "DatabaseSet binding shape" (the fan-out collapse this phase
> selectively opens).

## Why this phase exists

The serial read path is **complete and green**. As of this branch the
milox suite is **542 pass / 287 xfail**, and a measurement of the
redirect harness shows **~285 of the 287 remaining xfails are one
bucket**: the parallel per-proc-*unmerged* handler surface. Breakdown
(parallel-scope xfail / total per redirected module):

| module             | parallel xfail / total |
|--------------------|------------------------|
| `test_milidatabase`| 162 / 275 |
| `test_reductions`  | 55 / 106 |
| `test_adjacency`   | 40 / 51 |
| `test_derived`     | 28 / 82 |
| `test_miliinternal`| 0 / 41 |
| `test_projection`  | 0 / 5 |
| `test_reader`      | 0 / 7 |

Plus `test_grizinterface` (4 cases — **redirected + green as of
I.2**). Everything else (serial primal / reshape /
geometry / adjacency / reductions / derived value engine / projection)
is green.

## The core finding — the blocker is policy, not data

The per-fragment data is **fully retained in Rust**:

- `crates/mili-rs/src/family_set.rs` — `pub struct DatabaseSet { fragments: Vec<Database> }`,
  with **already-public** `fragment_count() -> usize`,
  `fragment(rank: usize) -> Option<&Database>`,
  `fragments() -> &[Database]` (around lines 159–169). Every other
  `DatabaseSet` accessor (`class_names`, `labels`, `query`,
  `state_maps`, `materials`, …, lines ~174–520) **merges** across
  fragments per `reductions.py` semantics.
- `crates/mili-py/src/database.rs` — `enum Backend { Single(Box<Database>), Set(Box<DatabaseSet>) }`
  (~line 27), chosen at open time by fragment count. Every
  `#[pymethods]` accessor matches on `Backend` and calls the
  **merged** `DatabaseSet` accessor for the `Set` arm.

So this is **not** a core data-model change and **not** a reversal of
decision 19's parity invariant — the per-fragment `Database`s are each
already bit-exact on the serial gate. It is an *additive* FFI + Python
wrapper-layer slice. What blocks it today is the deliberate **policy**
(decision 19 + m1.md decision-shape): `DatabaseSet` accessors merge,
and the redirect harness encodes that by xfailing every
parallel-handler class as "Rust DatabaseSet collapses the per-proc
fan-out".

## What the parallel suite actually needs (the upstream contract)

Upstream (`reference/mili-python/src/mili/`):

- `parallel.py:19-356` — `LoopWrapper` (suppress_parallel, serial over
  fragments) and `ServerWrapper` (subprocess per proc) both wrap a
  **list** of per-proc `_MiliInternal` and forward every public method
  as `[proc.method(*a, **kw) for proc in procs]` → a **per-proc list**.
- `reductions.py` — `combine` / `merge_result_dictionaries` /
  `list_concatenate_unique` / `reduce_*` (lines 16–168): the
  `merge_results=True` reduction of those per-proc lists back to a
  single merged shape.
- `milidatabase.py` — `MiliDatabase.__postprocess(results,
  reduce_function=reductions.combine)`: applies the reduce when
  `merge_results=True`, returns the raw per-proc list when `False`.
- `grizinterface.py` (213 LOC) + `tests/test_grizinterface.py`
  (46 LOC, 4 tests) — `GrizInterface.__init__` is built **entirely**
  around `merge_results=False`: `db.class_names()` →
  `List[List[str]]`, `self.processor_count = len(class_names)`,
  `db.state_maps()[0]`, `db.mesh_dimensions()[0]`,
  `db.srec_fmt_qty()[0]`, `db._mili.connectivity_ids()`,
  `db._mili.mesh_object_classes()`, `db._mili.subrecords()`,
  `db._mili.parameters()` (a per-proc list iterated in
  `merge_parameters` / `load_free_node_data`).

milox today: `crates/mili-py/python/milox/parallel.py` has
`LoopWrapper`/`ServerWrapper` as **marker `__getattr__` passthroughs**
over a single merged `PyMiliDatabase`; `milox.reader.open_database`
selects them by fragment count but they return the **merged** shape;
`MiliDatabase.__postprocess` is identity + return-code raising
(decision 19) and `milox.reductions.combine` is identity.

## Decision 20 (amends decision 19 — additive, not a reversal)

Recorded in [`m4.md`](m4.md) § "Decision 20". In brief: decision 19's
invariant (parity-sensitive value/topology math in the Rust core)
**stays intact** — Phase I adds **no new value math**. It exposes the
*already-computed* per-fragment results **before** the existing merge,
as a typed per-fragment FFI accessor surface; the merge stays in the
Rust `DatabaseSet` for `merge_results=True`. The amendment is narrow:
*milox MAY surface the per-fragment-unmerged shape for the
`merge_results=False` contract, sourced from the already-parity-correct
per-fragment core outputs.* This is the decision-18/19 precedent one
level out: per-proc *list assembly* is non-parity plumbing over
already-bit-exact per-fragment `Database` outputs.

## Phased plan (each its own parity-validated PR)

### Phase I.1 — per-fragment FFI accessors  ✅ **LANDED**

**Shape decision: option (a) — `*_per_fragment()` siblings returning
a per-fragment list** (1-element for the `Single` backend). Chosen
over the `fragment_view(rank)` handle because it is exactly the
`[proc.method(...) for proc in procs]` shape upstream's
`LoopWrapper`/`ServerWrapper` forwarding (I.2) consumes — the direct
FFI primitive for I.2 — whereas a borrowing handle would need PyO3
shared-ownership/lifetime gymnastics over `DatabaseSet`-owned
fragments for no contract benefit. Added to
`crates/mili-py/src/database.rs` (a new `frags()` helper over
`DatabaseSet::fragment(rank)`): `fragment_count`,
`{times,state_count,mesh_dimensions,srec_fmt_qty,class_names,
material_numbers,labels_of_class,labels,materials,parameters,
state_maps,mesh_object_classes,subrecords,nodes,connectivity_ids,
materials_of_class_name,parts_of_class_name,query}_per_fragment`
(`query_per_fragment` is primal-only — upstream's per-proc
`_MiliInternal.query` is primal; derived is the wrapper layer,
I.2/I.3 — with the `LoopWrapper` empty-entry leniency). No merge
logic touched. **Parity gate `parity_per_fragment.rs` green**;
milox suite unchanged at **542 / 287 xfail** (additive API + Rust
parity test, no behavior change; harness intentionally not narrowed
— promotion is I.4).

Original scope text (kept for I.2 reference): Add a per-fragment
read surface to `crates/mili-py/src/database.rs` backed by the
existing `DatabaseSet::fragment(rank)` / `fragment_count()`. The
`Single` backend returns a 1-element list (a serial db is a 1-proc
family — matches upstream `_MiliInternal` selection).

Accessor set the parallel suite touches (from the GrizInterface +
wrapper-class audit): `class_names`, `parameters`, `subrecords`,
`mesh_object_classes`, `connectivity_ids`, `state_maps`, `labels`,
`materials_of_class_name`, `parts_of_class_name`, `nodes`,
`mesh_dimensions`, `srec_fmt_qty`, `query` (+ whatever the
`_REDUCTIONS_WRAPPER_METHODS` / `_ADJ_PARALLEL_CLASSES` /
`_MDB_PARALLEL_CLASSES` method lists in
`crates/mili-py/tests/test_upstream_readpath.py` enumerate — that
file is the authoritative per-method scope list).

**Parity gate (new):** `crates/mili-rs/tests/parity_per_fragment.rs`
asserting each fragment view is bit-exact vs the upstream per-proc
`_MiliInternal` on `reference/mili-python/tests/data/parallel/d3samp6`
and `.../parallel/basic1` (the corpora the parallel tests use). Follow
the existing `parity_*.rs` pattern (skip-not-fail when the submodule /
oracle is absent — see `CLAUDE.md`).

### Phase I.2 — milox `LoopWrapper`/`ServerWrapper` per-proc forwarding  ✅ **LANDED**

The marker `__getattr__`-passthrough in
`crates/mili-py/python/milox/parallel.py` was replaced with the
upstream per-proc forwarding contract adapted onto the Phase-I.1
`*_per_fragment()` accessors (decision 20 shape (a):
`db.<m>_per_fragment(...)` *is* upstream's
`[proc.<m>(...) for proc in procs]`). `grizinterface.py` ported
**verbatim** into `crates/mili-py/python/milox/grizinterface.py`
(imports rebased `mili.*`→`.`; bodies byte-for-byte); wired into the
harness `_REDIRECT` (`"mili.grizinterface": milox.grizinterface`) +
`_REDIRECTED`. **Redirected `test_grizinterface` (4 cases) green;
milox 542 → 546 pass / 287 xfail unchanged.**

**Decision (recorded — the I.2/I.4 boundary, no surprise):** the
forwarding is **`merge_results`-gated** and **scoped to the
`GrizInterface.__init__` per-proc contract**, not a blanket
"every public method → per-proc":

- The wrapper now carries `merge_results` (threaded from
  `reader.open_database`). `merge_results=True` keeps the
  **merged** single-`Set` accessor (the Rust `DatabaseSet` already
  performed upstream's reduction — decision 19; the Python combine
  is identity). This is mandatory, not optional: the
  `test_reductions` `TestServerWrapperReductions` /
  `TestLoopWrapperReductions` classes open the *parallel* db with
  `merge_results=True` and their non-xfailed methods
  (`test_class_names`/`test_mesh_dimensions`/`test_srec_fmt_qty`/
  `test_state_maps`/…) compare it against the *serial* merged db —
  a blanket per-proc wrapper would regress those **already-green**
  cases (a 542 drop), which is why upstream's
  `MiliDatabase.__postprocess`-applies-`combine` split maps in milox
  to *the wrapper being merge-aware* (the I.3 re-reduce only moves
  *where* the `True` merge happens, never its result).
- Under `merge_results=False`, only the methods
  `GrizInterface.__init__` actually consumes per-proc
  (`class_names`, `state_maps`, `mesh_dimensions`, `srec_fmt_qty`,
  `parameters`, and the `db._mili.*` direct reads
  `connectivity_ids` / `mesh_object_classes` / `subrecords`) route
  to their `*_per_fragment()` sibling; every other field
  `GrizInterface` merely stores, so the merged shape satisfies it.
  Methods outside that set stay merged under `merge_results=False`
  too — so the standing `_MDB_PARALLEL_CLASSES`
  (`merge_results=False`) bucket does **not** incidentally flip:
  its `state_maps` / `mesh_object_classes` / `reload_state_maps`
  assertions still legitimately differ (raw per-proc dicts/tuples
  are not the upstream `StateMap` / `Dict[str,MeshObjectClass]`
  shape — verified: `_MDB_PARALLEL_CLASSES`/`_ADJ_PARALLEL_CLASSES`/
  `_REDUCTIONS_*` all hold, 287 xfail unmoved). The remaining
  per-proc accessor surface + the xfail-bucket promotion is **I.4**;
  the `merge_results=True` re-reduce relocation is **I.3** — exactly
  the phased split this doc already prescribes.

### Phase I.3 — `merge_results=True` re-reduce (highest risk)

Port `reductions.combine` / `merge_result_dictionaries` / `reduce_*`
verbatim into `milox.reductions` (currently identity) and make
`MiliDatabase.__postprocess` / the accessors re-reduce the per-proc
lists when `merge_results=True`. **Decision point (record in I.3):**
keep the Rust merge as the `merge_results=True` path (don't
double-work) and only Python-merge where a test needs the per-proc
list — re-assess against the `_REDUCTIONS_WRAPPER_CLASSES`
serial-vs-parallel comparisons. **Net `merge_results=True` behavior
must be unchanged** — gate behind the already-green merged suite
staying green.

### Phase I.4 — promote the parallel-handler xfail buckets

In `crates/mili-py/tests/test_upstream_readpath.py`, promote each as
its Phase-I.x dependency lands (strict-xfail removed only when
bit-exact): `_MDB_PARALLEL_CLASSES`, `_ADJ_PARALLEL_CLASSES`,
`_REDUCTIONS_WRAPPER_CLASSES` / `_REDUCTIONS_COMBINE_CLASS` /
`_REDUCTIONS_MERGEDF_CLASS`, `ParallelDerivedExpressions`. Whatever
remains genuinely parallel-only (e.g. `use_shared_memory` subprocess
semantics with no serial oracle) stays honestly xfailed with a
concrete reason — never silently passed (the harness is strict: a
passing xfailed case fails the harness until promoted).

## Out of Phase I (unchanged boundary)

- `*_alt` griz strain variants (`prin_strain*_alt` /
  `prin_dev_strain*_alt`) — listing-only, **no value-test** forcing
  function; not implemented speculatively.
- Phase 3 write path (`test_append_states*`, `test_modify_database`,
  the write halves) — unchanged; harness marks
  `xfail(reason="Phase 3 write path")`.

## Reproducible environment / commands

```
scripts/setup-parity.sh                       # inits submodules + pip-installs the mili oracle (idempotent)
pip install pytest ./crates/mili-py           # maturin-builds + installs milox (rebuild after Rust changes)
python -m pytest -q crates/mili-py/tests      # the milox parity + redirect suite (expect 542 pass / 287 xfail at I.0)
cargo test --workspace --exclude mili-py --features parity   # Rust incl. parity_*.rs
cargo fmt --check && cargo clippy --workspace --exclude mili-py --features parity
```

Run a single redirected module/case:
`python -m pytest -q crates/mili-py/tests/test_upstream_readpath.py -k "grizinterface"`.
CI job is `test-milox` in `.github/workflows/ci.yml`
(`pip install pytest ./crates/mili-py` then `pytest -q crates/mili-py/tests`).

## Key files (the starting-point map)

| Concern | Path |
|---|---|
| DatabaseSet + per-fragment accessors | `crates/mili-rs/src/family_set.rs` |
| FFI Backend dispatch | `crates/mili-py/src/database.rs` |
| Wrapper markers (to replace, I.2) | `crates/mili-py/python/milox/parallel.py` |
| Handler selection | `crates/mili-py/python/milox/reader.py` |
| `__postprocess` identity (I.3) | `crates/mili-py/python/milox/milidatabase.py` |
| `combine` identity (I.3) | `crates/mili-py/python/milox/reductions.py` |
| Redirect harness + xfail buckets | `crates/mili-py/tests/test_upstream_readpath.py` |
| Upstream parallel contract | `reference/mili-python/src/mili/parallel.py`, `reductions.py`, `grizinterface.py` |
| Upstream test | `reference/mili-python/tests/test_grizinterface.py` |
| Tracker (bump counts here) | `planning/mili-rs/status.md` (milox row), `m4.md` § Phase I |

## Update protocol

Each Phase I.x lands as its own commit/PR with: the parity gate green,
the milox suite count bumped in `planning/mili-rs/status.md` and
`m4.md` § Phase I, any corpus-vs-doc surprise logged per
`planning/mili-rs/status.md` § "Surprises", and the harness xfail set
narrowed (never a silent pass).
