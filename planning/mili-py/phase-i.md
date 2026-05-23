# `mili-py` Phase I — landed (parallel per-proc-unmerged surface)

> **Status: ✅ COMPLETE.** Full decision history and implementation
> notes live in [`m4.md`](m4.md) (decisions 20–21). This file is
> retained for cross-references; the body has been collapsed.

## What landed

- **I.1 — per-fragment FFI accessors.** A `*_per_fragment()` sibling
  surface on `PyMiliDatabase` (returning a per-fragment list,
  1-element for the `Single` backend) over the already-public
  `DatabaseSet::fragment(rank)` / `fragment_count()`. No merge logic
  touched; additive only. Gated by
  `crates/mili-rs/tests/parity_per_fragment.rs`.
- **I.2 — `LoopWrapper`/`ServerWrapper` per-proc forwarding.**
  `merge_results`-gated forwarding scoped to the `GrizInterface`
  per-proc contract; `grizinterface.py` ported verbatim into
  `crates/mili-py/python/milox/grizinterface.py`. Redirected
  `test_grizinterface` (4 cases) green.
- **I.3 — `merge_results=True` re-reduce relocation.** Wrapper's
  `True` arm forwards through a `_MiliInternal` adapter over the
  Set-backed `PyMiliDatabase`; the Rust `DatabaseSet` merge stays
  the single source of merged truth (no double-work). Promoted the
  cross-fragment-merged `_REDUCTIONS_WRAPPER_METHODS` (+18 cases);
  fixed one parity-correct core gap (`superclass_from_class_name`
  scans fragments first-hit-wins, matching upstream
  `reduce_superclass_from_class_names`).
- **I.4 — full parallel-handler surface.** Adopted upstream's
  contract verbatim: real per-proc list of milox `_MiliInternal`
  (each opening one fragment's A-file via `open_single`), generic
  `__getattr__` forwarder + name→reducer dispatch over the verbatim
  `milox.reductions`. `milox.adjacency` ported verbatim. Two
  faithful-contract fixes (`_MiliInternal.query` swallows
  `MiliPythonError` like upstream `__query`; `material_numbers`
  returns an ndarray).
- **Net.** milox **542 → 827 pass / 287 → 6 xfail**; the only
  remaining xfails were the Phase-3 write path.

## Gating tests

- `crates/mili-rs/tests/parity_per_fragment.rs` (I.1 core gate).
- `crates/mili-py/tests/test_upstream_readpath.py` (redirect harness;
  the parallel-handler xfail buckets `_MDB_PARALLEL_CLASSES` /
  `_ADJ_PARALLEL_CLASSES` / `_REDUCTIONS_WRAPPER_*` /
  `_REDUCTIONS_COMBINE_CLASS` / `_REDUCTIONS_MERGEDF_CLASS` /
  `ParallelDerivedExpressions` all promoted by close of I.4).

## Decisions

- Decisions 20 (additive amendment to decision 19 — milox MAY
  surface the per-fragment-unmerged shape for the
  `merge_results=False` contract) and 21 (the I.4 architecture
  point) recorded in [`m4.md`](m4.md).
