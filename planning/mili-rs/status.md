# `mili-rs` implementation status

Live tracker. **Source of truth for what's safe to depend on**; the
design rationale lives in [`plan.md`](plan.md).

## What's next

**Phase 1 is complete** for the single-A-file read path. Before
starting Phase 2 (`mili-py`), we firm up the Rust layer with three
items so we don't end up patching two layers in lockstep. See
**Phase 1.5** below.

The one Phase-1 follow-up that can't be closed from a normal PR:

- **Step 13 fuzz cron clean-run gate.** CI job exists
  (`.github/workflows/ci.yml § fuzz`, scheduled `0 7 * * *` UTC). Flips
  to ✅ once a maintainer confirms the first cron run lands clean (or
  manually triggers `workflow_dispatch` from the Actions UI). No code
  change required from here.

## Phase 1.5 — pre-`mili-py` read-side firming

Three items to land before starting `mili-py` M1. Each closes a gap
we found while auditing what the Rust layer covers vs. what the C
library and the upstream test corpora cover.

| Step | Lands                                                                | Status |
|-----:|----------------------------------------------------------------------|:-------|
| 17   | Multi-A-file orchestration in Rust (`DatabaseSet`, parallel open)    | ✅ done |
| 18   | C-library + `xmilics` parity coverage and end-to-end reader smoke    | ✅ done |
| 19   | numpy/rayon integration plan pinned in `plan.md` § FFI integration plan | ✅ done |

**Step 17 — `DatabaseSet` (multi-A-file orchestration in Rust).**
MPI-segmented runs write one mili family per rank (`run.plt000A`,
`run.plt001A`, …). The C library opens each as a separate family;
mili-python orchestrates the fan-out in `LoopWrapper` /
`ServerWrapper`. We move that orchestration into Rust so the
multi-fragment open path can use rayon over fragments and so the
mili-py binding doesn't need a parallel-wrapper layer on top.

Concretely:

- New module `crates/mili-rs/src/family_set.rs`.
- `DatabaseSet::open(base: &Path)` — discover fragments via the
  mili-python regex (`re.escape(base) + r"(\d*)A$"`,
  `reference/mili-python/src/mili/afileIO.py:34-57`), open them in
  parallel with `rayon::par_iter`.
- Same accessor surface as `Database` (`labels`, `times`,
  `class_names`, `nodes`, `connectivity`, `state_maps`, `query`).
- Merge semantics match `mili-python/src/mili/reductions.py`
  (`list_concatenate_unique`, `dictionary_merge_no_concat`).
- Single-fragment paths still go through `Database` directly;
  `DatabaseSet::open` on a 1-fragment base returns a thin wrapper.

**Step 18 — C-library + `xmilics` parity coverage.** The
`reference/mili` submodule (initialized via `git submodule update
--init reference/mili`) ships test fixtures the mili-python corpus
doesn't have, plus baseline files for byte-level verification.

Concretely:

- Add `crates/mili-rs/tests/parity_xmilics.rs`. Cover the multi-proc
  inputs from `reference/mili/test/xmilics/`: `d3samp6` (8 procs),
  `bar1` (8 procs), `shell_mat2` (11 procs). Open each fragment
  individually, diff one representative query per fragment against
  mili-python reading the same fragment.
- Add `crates/mili-rs/tests/smoke_full_corpus.rs`. `reader.c`-style:
  for every fixture in both `reference/mili-python/tests/data/` and
  `reference/mili/test/`, open + enumerate all classes, svars,
  states, read every state. No oracle — just "doesn't panic, counts
  add up." Catches the long tail across all 14 serial + 2 parallel
  + 3 v3 + 9 xmilics fixtures.
- Once Step 17 lands, add a `DatabaseSet`-level row in the parity
  matrix for `d3samp6` (the most-used parallel fixture).
- Both new test files gate on submodule presence the same way the
  existing fixture tests do (early-return when corpus absent).

**Step 19 — pin the numpy/rayon plan in `plan.md`.** No Rust code
changes; this is documentation so `mili-py` M1 starts from a
settled design. Updates land in `plan.md` § "FFI integration plan"
(new section). Captures:

- `pyo3::Python::allow_threads` wraps every `Database::query`
  invocation in the binding; rayon's `par_chunks_mut` runs with
  the GIL released.
- `IntoPyArray::into_pyarray_bound` is the default zero-FFI-copy
  return path — moves the owned `Vec<T>` into a numpy array, no
  byte copy at the FFI boundary.
- `Arc<Mmap>`-backed `PyCapsule` zero-decode path is deferred to
  Phase 2 M5 (profiling-driven). Only pays off for single-slab,
  aligned, native-endian queries.

Everything below is reference material for future work.

## Phase 1 — step status

| Step | Lands                                                          | Status     |
|-----:|----------------------------------------------------------------|:-----------|
| 0    | Workspace, CI skeleton, `MiliError`, fixture symlinks          | ✅ done    |
| 1    | `header.rs` + golden bytes from `basic1.pltA`                  | ✅ done    |
| 2    | `directory.rs` (v3 + v2; v1 deferred with typed error)         | ✅ done    |
| 3    | `param.rs` scalar/string/array decode; `ti.rs` open stub       | ✅ done    |
| 4    | `family.rs` open path, `state_map` resolution, end-marker      | ✅ done    |
| 5    | `mesh.rs`: `CLASS_DEF` + `CLASS_IDENTS`, nodes, connectivity   | ✅ done    |
| 6    | high-level TI accessors (labels, materials, element_sets)      | ✅ done    |
| 7    | `svar.rs`, `srec.rs`, `derive_lumps`                           | ✅ done    |
| 8    | `buffer.rs` and `endian.rs` — **Phase 1 exit**                 | ✅ done    |
| 9    | `query.rs` single-svar single-state, `RESULT_ORDERED`          | ✅ done    |
| 10   | `query.rs` full filter set, `OBJECT_ORDERED`, vec_array        | ✅ done    |
| 11   | array-svar subscript notation (`"hx[3]"`, 1-based)             | ✅ done    |
| 12   | rayon over states; criterion benches (≥ 2× mili-python gate met) | ✅ done  |
| 13   | cargo-fuzz on `directory.rs`, `header.rs`, `param.rs`          | 🟡 cron pending |
| 14   | pyo3 cross-impl parity harness                                 | ✅ done    |
| 15   | nightly fuzz CI cron + planning-doc fix-ups                    | ✅ done    |
| 16   | Phase-1 closeout (corpus-wide parity, IP-count contract, API audit) | ✅ done |

**Phase 1 exit gate (Step 8) landed.** Step 12 cleared its ≥ 2×
mili-python throughput gate: ~4.7× single-svar / ~8× multi-svar on
basic1 via the pyo3 baseline bench. Step 13 stays 🟡 only because the
clean-run gate is time-based.

**Phase 1.5 closeout decisions (Step 17).** The plan called for some
cross-fragment behavior that the mili-python reference doesn't
actually enforce. Resolved during implementation:

- *Connectivity merge does **not** remap node ids.* `reduce_connectivity`
  is plain `list_concatenate` — each fragment's connectivity references
  its own local node space, and remapping would corrupt ghost-layer
  rows. `DatabaseSet::connectivity` matches this and the docstring
  flags it.
- *`FragmentMismatch` is narrow.* Only the time axis (state count +
  per-state f32 bit-pattern) is checked at open time. Divergent svar /
  class metadata is per-call: a fragment that doesn't declare the
  class or doesn't carry the svar on the class silently contributes
  zero rows (mirrors `LoopWrapper`'s try/except).
- *No path normalization.* The user passes the literal base
  (`basic1.plt`, `d3samp6.th`) — same contract as
  `mili.reader.open_database` after its `os.path.basename` step. No
  trailing-`A` / digit / `.plt` stripping inside `DatabaseSet::open`.
- *Query labels surfaced through `Database::query_with_labels`.*
  Needed by `DatabaseSet` to merge along the entity axis. Old
  `Database::query` is preserved as a thin wrapper that discards the
  labels vector.

## Test coverage snapshot

| Suite                          | Tests | Notes |
|--------------------------------|------:|:------|
| `cargo test --workspace`       | 205   | unit + fixture integration (+ 6 `DatabaseSet` fixture rows, + 7 `family_set` unit tests, + 1 corpus-wide smoke walker) |
| mili-python parity (`pyo3`)    | 22    | 12 corpus fixtures bit-exact; xmilics per-fragment + d3samp6 set-level parity; `cargo test --features parity` |
| cargo-fuzz (nightly cron)      | 3     | header, directory, param targets |
| Criterion benches              | 4     | open, nodes, query_single, query_many (+ `mili_python_baseline` under `--features parity`) |

## Mandatory edge-case tests (per `plan.md`)

All seven from `test_bugfixes.py` are accounted for:

| # | Test                                          | Status |
|--:|-----------------------------------------------|:-------|
| 1 | Non-sequential mesh-object blocks coalesce    | ✅ Step 5 |
| 2 | Double-precision nodal positions              | ✅ Step 10 |
| 3 | Vec-array with mixed component widths         | ✅ Step 10 (d3samp4 `es_1a`) |
| 4 | Inconsistent IP counts across subrecords      | ✅ Step 16 (typed `MiliError::InconsistentIpCounts` contract; activates once Phase-2 element-set substitution lands) |
| 5 | Array-svar subscript notation                 | ✅ Step 11 (d3samp6.thA `hx[3]` bit-exact) |
| 6 | `dir_version_2` fixture                       | ✅ Step 2 |
| 7 | State end marker `~` round-trip               | ✅ read; write deferred to Phase 3 |

## Known gaps Phase 2 inherits

The Rust core is honest about what it does and doesn't cover. Phase 2
will run into these — none of them block Phase 2 from starting, but
the bindings layer should surface the typed errors cleanly. Phase 1.5
closes the multi-A-file gap before Phase 2; the others remain.

- **Aggregate VEC_ARRAY queries** (e.g. `db.query("es_1a", "shell")`)
  work in mili-rs end-to-end, but mili-python raises `IndexError` on
  the same query, so we can't cross-validate. Resolution path: either
  upstream Python patch, or a fixture that exercises both readers via
  component-name lookup. Pinned in `query.rs::find_vector_parent` (VEC_ARRAY
  parents intentionally skipped).
- **Bare-component lookup with cross-material element-sets.**
  `db.query("sx", "brick")` on basic1 currently returns
  `MiliError::LabelNotFound` for material 5 / 7 labels because their
  element-sets (`es_5a`, `es_7a`) aren't substituted as parents of
  `sx`. mili-python substitutes them. Once Phase 2's binding wraps the
  query path, this becomes a visible gap. Resolution: extend
  `find_vector_parent` to consider VEC_ARRAY parents per-subrec; the
  `InconsistentIpCounts` typed error (Step 16) is the contract that
  fires when the substituted parents disagree on IP count.
- **Partial-dim array subscript** (e.g. `g[1]` on a `dims=[3,4]`
  svar). Surfaces as `MiliError::Unsupported`. No corpus fixture has
  multi-D array svars. Implement when a real call hits it.
- **Cross-srec-format multi-state query.** Surfaces as
  `MiliError::Unsupported`. Every corpus fixture has `srec_fmt_qty=1`.

## Deferred to Phase 3 (write path)

Typed errors today; not in Phase 2 scope.

- Write path (everything under `mc_wrt_*` / `mc_new_state`).
- Directory v1 (`MiliError::UnsupportedDir(1)`).
- `SURFACE_CONNS` payload decode.
- `block_obj_fmt` connectivity (`list_obj_fmt` is universal in our corpus).
- mmap-on-NFS / Lustre `pread` fallback (no benchmark motivates it).

## Surprises worth remembering

Brief pointers — each has a fix in the code and the byte-layout docs
were updated to match. Read the linked source for the full story.

- **`CLASS_DEF` superclass is in `MODIFIER2`, not `MODIFIER1`** —
  `entry-payloads.md § CLASS_DEF`, `mesh.rs::add_class_def`.
- **`CLASS_DEF` `long_name` re-declaration may disagree** (cosmetic;
  superclass is the load-bearing field) — `mesh.rs::add_class_def`.
- **State files carry an 8-byte per-state header** (i32 srec_id + f32
  time) — `format.md § File set`, `query.rs::state_data_start` skip.
- **VEC_ARRAY inner order is components-fastest, IP-slowest** —
  `format.md § Subrecord byte-layout matrix`.
- **`M_MESH`-superclass subrecs carry one object per state even with
  `qty_id_blks=0`** — `srec.rs::patch_m_mesh_classes`,
  `entry-payloads.md § STATE_REC_DATA`.
- **`PREC_LIMIT_DOUBLE` leaves `M_FLOAT` 4 bytes** — only explicit
  `M_FLOAT8` svars are 8 bytes. `format.md § Numeric types`.

## Open questions (still active)

Surfaced in `plan.md § Open questions`; none block Phase 2 start:

- `block_obj_fmt` connectivity prevalence — defer until a fixture trips it.
- mmap on Lustre / NFS — defer the pread fallback until a benchmark motivates it.
- UTF-8 strictness — currently strict; downgrade to lossy behind a feature flag if a real fixture breaks.
- Element-set name → material id parse rule — Step 6 currently maps only sets whose name parses as `i32`. Revisit on non-integer setname.
- Aggregate VEC_ARRAY parity oracle — see "Known gaps Phase 2 inherits".

## Reference: mili C library test corpus

The `reference/mili` submodule (not auto-checked-out — run
`git submodule update --init reference/mili` once) ships its own
test corpus and baseline files. Key facts for Phase 1.5:

**Layout** (`reference/mili/test/`):

- `mili/mili_C_tests/` (12 tests, all `num_procs: 1`). Mostly
  write-path: `mixdb_wrt_stream/subrec/mixed_subrecords`, `restart*`
  (5 variants), `state_write_check`, `value_change`, `del_test`. The
  read-relevant sources are `reader.c` (round-trip read-then-write),
  `titest.c` (TI table iteration), `mode_test.c`, and `MiliDiff.c`
  (binary diff utility for baselines).
- `mili/version_3_mili_C_tests/` (12 tests). Same suite for the v3
  directory format.
- `mili/mili_fortran_tests/` (~12 tests). Fortran-binding tests; same
  library, different binding. No new Rust coverage.
- `xmilics/` (9 fixtures). Multi-proc MPI-segmented inputs — these
  are the high-value fixtures for Phase 1.5 Step 18: `d3samp6`
  (8 procs), `bar1` (8 procs), `shell_mat2` (11 procs), `basic2`,
  `cylinder`, `cylinder_4hex`, `ml40`, `d3samp6_tfile`, `bar5`.
- Test definitions: `mili_test_definitions.py`,
  `xmilics_test_definitions.py`. Each test has a `.baseline` file
  (binary expected output) consumed by `MiliDiff.c`.

**What the C library does with MPI fragments** (relevant to Step 17):

- `mc_open` opens *one* family at a time (`reference/mili/src/mili.c:445`).
  No combined-view API in the library itself.
- The `nproc` scalar param tells you the fragment count
  (`reference/mili/src/mili.c:1328`). If absent, `find_proc_count`
  globs the directory (`reference/mili/src/mili_util.c:1488`).
- The `xmilics` CLI is the offline combiner — reads all fragments,
  writes one merged plot file. Phase 3 territory if we port it.

## Reference: rust-numpy + rayon integration shape

(Pinned here so Phase 1.5 Step 19's `plan.md` update has a
single-source-of-truth checklist.)

`pyo3` and `rust-numpy` compose cleanly with `rayon`:

```rust
#[pyfunction]
fn query<'py>(py: Python<'py>, /* args */)
    -> PyResult<Bound<'py, PyArray1<f32>>>
{
    let v: Vec<f32> = py.allow_threads(|| {
        db.query(args)        // internal rayon::par_chunks_mut here
    })?;
    Ok(v.into_pyarray_bound(py))
}
```

- `Python::allow_threads` releases the GIL for the duration of the
  rayon section. Required so other Python threads can run while we
  parallelize state gather.
- `IntoPyArray::into_pyarray_bound(py)` moves an owned `Vec<T>` (or
  `ndarray::Array<T,_>`) into a numpy array — numpy adopts the
  allocation; no byte copy. This is the default return path.
- `ToPyArray::to_pyarray_bound(py)` copies (when ownership can't
  transfer, e.g., `&[T]`). Avoid on the hot path.
- `Arc<Mmap>` + `PyCapsule` for true zero-decode views: deferred to
  Phase 2 M5. Only wins on aligned + native-endian + single-slab
  queries (rare in practice).

## How to update this file

1. When a Phase-1.5 or Phase-2 step lands, flip the row to ✅ in the
   appropriate section. Don't re-open closed Phase-1 rows.
2. If a closed step regresses, demote it to 🟡 with the failing test
   linked — don't paper over with notes.
3. New surprises go under "Surprises worth remembering" with a one-line
   pointer and a code reference. Keep retrospectives in the commit
   message, not here.
