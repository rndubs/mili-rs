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

## Resolved: deferred query-merge parity gap (was a real bug)

The xmilics `DatabaseSet` parity row deferred in PR #13 (only the
time axis was asserted) is **closed**, and closing it surfaced a real
correctness bug shipped in PR #13 — not the dedup-heuristic nuance
that was originally suspected.

**Root cause.** `query.rs::gather_all` (the no-filter query path)
built the entity axis from the subrecord's `id_blocks`, i.e. 1-based
mesh-object ids, *not* user-facing labels. mili-python maps those
through the class label array (`miliinternal.py:1297`,
`class_labels[ordinals_in_srec]`). Single-fragment `Database::query`
discards the label vector so this was invisible, but
`DatabaseSet::query` merges and dedupes on it. On d3samp6 (8 frags),
each fragment emitted ordinals `~1..35`; `dedupe_first` collapsed the
220-row concatenation (144 truly-unique nodes) to **40 colliding
ordinals** with first-fragment values winning — semantically corrupt.
The existing `database_set_fixtures` test only checked
`DatabaseSet::query` against `query_with_labels` self-consistently
(both used the buggy ordinals), so it stayed green.

**Fix.** `query_with_labels` now maps MO ids → real labels via the
class `Labels` TI param (`Database::map_mo_ids_to_labels`), exactly
mirroring `miliinternal.py:1297`; classes with no `Labels` param keep
identity (matches `miliinternal.py:281`). The filtered path
(`gather_by_labels`) already emitted real labels and is unchanged.

**Secondary fix (the originally-suspected nuance).**
`DatabaseSet::query`'s dedup now replicates
`merge_result_dictionaries`'s post-pass exactly (`reductions.py:72`):
`np.unique(labels, return_index=True, return_counts=True)` sorts, so
when any label repeats across fragments the merged entity axis is
reordered to **ascending-unique** order taking each label's first
occurrence; with no duplicates the raw concatenation order is kept
(it is *not* sorted). Implemented in `family_set.rs::merge_unique`.

**Parity status.** `tests/parity_xmilics.rs::parity_d3samp6_set_query_nodpos`
asserts `DatabaseSet::query("nodpos","node")` is **bit-exact**
(entity-axis labels *and* flat values) against mili-python's merged
`LoopWrapper` query on the 8-fragment d3samp6 family across sampled
states. Green.

## Test coverage snapshot

| Suite                          | Tests | Notes |
|--------------------------------|------:|:------|
| `cargo test --workspace`       | 206   | unit + fixture integration (+ 6 `DatabaseSet` fixture rows, + 8 `family_set` unit tests, + 1 corpus-wide smoke walker) |
| mili-python parity (`pyo3`)    | 24    | 12 corpus fixtures bit-exact; xmilics per-fragment + d3samp6 set-level **state-axis and query-merge** parity (bit-exact); **+ `parity_reshape` — every Phase-G `_MiliInternal` reshape bit-exact vs. the upstream oracle across the full serial corpus**; `scripts/setup-parity.sh` then `cargo test --workspace --exclude mili-py --features parity` |
| `milox` parity + redirect      | 399 + 186 xfail | `import milox` vs upstream `mili`. M1 metadata (171) + M2 (51) + M3 (17) + M4 Slice A/B (bit-exact) + **M4-followup Phase F** skeleton + **Phase G**: the primal-only `_MiliInternal` reshape surface (logic in Rust core `mili_rs::reshape`, thin `database.rs` binding; `StateVariable`/`Subrecord`/`MeshObjectClass`/`StateMap`/`MiliType` verbatim-ported to `milox.datatypes`) **+ the `test_milidatabase.py` read half closed**: `MiliDatabase` wrapper does return-code raising (`parse_return_codes`) + mdg-enum arg coercion, `_MiliInternal` gained upstream-signature `query`/`connectivity` input validation, and the Rust core query gained **named-component subscript** (`nodpos[ux]`/`stress[sy]`) + **multi-parent bare-component disambiguation by subrec membership** (`sx` on `brick`), each bit-exact vs the upstream oracle (`tests/parity_component_subscript.rs`). **+ Phase H geometry sub-slice**: `connectivity_ids`, `nodes_of_elems`, `nodes_of_material`, `faces`, `measure` in the Rust core (`mili_rs::geometry`, thin `database.rs`/`miliinternal.py` bindings), bit-exact vs the upstream `_MiliInternal` oracle across the serial corpus (`tests/parity_geometry.rs`); 15 `test_miliinternal`/`test_milidatabase` xfails promoted to green. Import-redirect harness (`test_upstream_readpath.py`) runs upstream `test_reader.py` (7) **+ `test_miliinternal.py` (37 pass, 4 Phase-H `xfail`) + `test_milidatabase.py` read half** green; the parallel handler classes + the remaining derived/projection/result-modifier serial methods are honest strict `xfail` (Phase H / parallel scope). Dedicated `test-milox` CI job; `mili-py` excluded from default `cargo test --workspace` |
| cargo-fuzz (nightly cron)      | 3     | header, directory, param targets |
| Criterion benches              | 4     | open, nodes, query_single, query_many (+ `mili_python_baseline` under `--features parity`) |

## Mandatory edge-case tests (per `plan.md`)

All seven from `test_bugfixes.py` are accounted for:

| # | Test                                          | Status |
|--:|-----------------------------------------------|:-------|
| 1 | Non-sequential mesh-object blocks coalesce    | ✅ Step 5 |
| 2 | Double-precision nodal positions              | ✅ Step 10 |
| 3 | Vec-array with mixed component widths         | ✅ Step 10 (d3samp4 `es_1a`) |
| 4 | Inconsistent IP counts across subrecords      | ✅ Step 16 contract; **active** since the mili-py M4-followup Slice B element-set substitution landed (serial/basic1 `sx`/`brick` mat 5 = 8 IPs vs mat 7 = 9 IPs raises `InconsistentIpCounts`) |
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
  the same query, so we can't cross-validate the *aggregate* form.
  Pinned in `query.rs::find_vector_parent` (VEC_ARRAY parents
  intentionally skipped). **M4 update:** the *component* path
  (`sx`/`sy`/`eps` on the class) **is** answerable by upstream — the
  oracle is `test_bugfixes.py` (exact values), so the component-level
  resolution is tractable and is now **CLOSED** by the mili-py
  M4-followup Slice B (below); only the bare *aggregate* form stays
  oracle-blocked (the existing coherent-data behaviour is kept, not
  chased).
- **Bare-component lookup with cross-material element-sets — CLOSED
  (mili-py M4-followup Slice B).** `db.query("sx","brick")` on
  serial/basic1 and `eps`/`sy` component-of-VEC_ARRAY on d3samp4 now
  resolve via the new core svar→element-set→IP-label linkage
  (`query::IntPoints`, `Database::build_int_points` mirroring upstream
  `__int_points`), VEC_ARRAY-parent substitution in
  `plan_state_svar_ip` / `try_vec_array_substitution` (per-subrec
  component-outer/IP-inner pickers), `ips=` *label*→positional-index
  mapping, the `f"{comp} ipt. {label}"` component naming, and the
  cross-material per-subrec IP-count reconciliation firing the Step-16
  `InconsistentIpCounts`. Bit-exact vs upstream
  (`test_query_parity.py::test_bugfixes_slice_b_oracle` (4),
  `_component_names`, `_cross_material_inconsistent_ips_contract`).
  Historical note: this needed four coupled core subsystems
  (VEC_ARRAY-parent resolution + a new svar→element-set→IP-label
  linkage with no current `mili-rs` analogue + ip-label→index mapping
  + cross-material per-subrec IP-count reconciliation), not the
  single `find_vector_parent` extension first assumed.
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

- **Bare component of a multi-parent VECTOR resolves by *subrec
  membership*, not global uniqueness** — `sx` is a component of
  `stress`, `stress_mid`, `stress_in`, `stress_out` (all VECTORs).
  The old `find_vector_parent` bailed (`None`) on >1 parent, so
  `query("sx","brick")` wrongly raised `NoMatchingSubrec` even though
  upstream returns it (the brick subrec carries `stress`). Fix:
  `find_vector_parents` returns every candidate; the planner picks the
  first whose subrecs match the queried class — upstream resolves the
  component via the subrec that carries its parent
  (`miliinternal.py:1222-1231`). `query.rs`; bit-exact in
  `parity_component_subscript.rs`. Found by `milox` Phase-G
  `test_milidatabase` read-half parity.
- **`parent[comp]` is a *named-component* query, not an integer
  subscript** — `nodpos[ux]` / `stress[sy]`: when the `[...]` content
  isn't all-integer it's a component-svar-name lookup of the VECTOR /
  VEC_ARRAY parent (`miliinternal.py:976-996`). New
  `QueryName::CompSubscript`; the planner rewrites it to the bare
  component gather, the result is keyed by the raw input and titled
  with the **parent's** title (`svar_query_meta`). `query.rs` /
  `family.rs`; bit-exact in `parity_component_subscript.rs`. Found by
  `milox` Phase-G `test_milidatabase` read-half parity.
- **`measure` is upstream a `MiliDatabase` method but lands on
  `_MiliInternal` in milox, as self-contained centroid geometry —
  *not* the derived engine.** Upstream `MiliDatabase.measure`
  (`milidatabase.py:882`) routes through `query("centroid",…)` +
  `reductions.combine`, and `centroid` is a `derived.py` expression.
  The Phase-H geometry sub-slice deliberately excludes the derived
  engine, so `measure` is implemented in `mili_rs::geometry` as the
  *self-contained* `__compute_centroid` arithmetic (`derived.py:1962`:
  NODE → its `nodpos`; element → mean of its first `node_count` node
  positions, BEAM dropping its 3rd node) over the already
  parity-correct primal `nodpos` query — no `centroid` derived var, no
  `reductions`. milox forwards engine attrs through
  `MiliDatabase.__getattr__`, so placing `measure` on `_MiliInternal`
  is wire-compatible. Validated by the redirected `test_measure`
  (hard-coded upstream distances; the four geometry `_MiliInternal`
  methods are bit-exact vs the oracle in `parity_geometry.rs`).
- **`__conns_ids` is fortran-1-based node ids minus 1, material kept
  raw** — upstream `connectivity_ids` (`miliinternal.py:213-218`) is
  `elem_conn[:,:-1]` (drop the trailing `part`) with the node columns
  `-= 1` and the `material` column left verbatim;
  `Database::connectivity_ids` mirrors this exactly (the labels
  variant maps the same `fid-1` through `node_labels`). Found while
  porting the Phase-H geometry sub-slice; bit-exact in
  `parity_geometry.rs`.
- **`element_sets()` key is `es_<n>`, not `<n>`** — upstream keys by
  `sname[sname.find('es_'):]` (`miliinternal.py:113-115`), so strip
  only `IntLabel_`. `integration_points()` then keys by `eset[-1:]`
  (last char) — `family.rs::{element_sets,integration_points}`. Found
  by `milox` M1 parity.
- **`labels()` for ident-only classes (`mat`/`glob`/`lcurve`) comes
  from `CLASS_IDENTS`, not a TI param** — upstream seeds `__labels`
  from every `CLASS_IDENTS` class as `arange(start, stop+1)`
  (`miliinternal.py:198-202`). `family.rs::labels` falls back to
  `ObjectClass::id_blocks` when no TI "Element/Node Labels" param
  matches. Found by `milox` M1 parity.
- **`DatabaseSet` metadata reductions are per-accessor, not all
  rank-0** — only `mesh_dimensions`/`materials`/`times`/`state_count`/
  `state_maps` are `zeroth_entry`; `material_numbers` is
  `list_concatenate_unique` and `element_sets`/`integration_points`
  are `dictionary_merge_no_concat` (`milidatabase.py`). See
  `family_set.rs` and `m1.md` resolved decision 8.
- **Upstream `connectivity()` is label-substituted, not the raw
  stream** — `miliinternal.py:217-223` drops the trailing part
  column, keeps the raw material number, and replaces each fortran
  node id with `node_labels[id-1]`. Distinct from raw
  `Connectivity::to_i32_vec` / `connectivity_ids`. Core gained
  `Database::connectivity_labels` /
  `DatabaseSet::connectivity_labels`; the raw borrowed primitive is
  unchanged. Found by `milox` M2 parity. (`m2.md` decisions 9, 11.)
- **Multi-fragment `nodes()` is dedup-by-label, not plain concat** —
  upstream `milidatabase.py:167-185` concatenates per-fragment node
  labels, keeps the first occurrence of each unique label in
  rank-concat order, returns those coordinate rows. The pre-M2
  `DatabaseSet::nodes` plain concat was a multi-fragment parity bug;
  fixed in `DatabaseSet::node_coords`. `Database::node_coords` also
  concatenates **all** NODES entries, not just the first. Found by
  `milox` M2 parity. (`m2.md` decision 10.)
- **`CLASS_DEF` superclass is in `MODIFIER2`, not `MODIFIER1`** —
  `entry-payloads.md § CLASS_DEF`, `mesh.rs::add_class_def`.
- **`CLASS_DEF` `long_name` re-declaration is _last-wins_**, not
  first-wins — upstream stores CLASS_DEF in a dict keyed by short name
  (`miliinternal.py:208-210`) so a later entry overwrites the earlier
  (`labeling`: `particle` is `"Nodal"` then `"Particles"`). Was a real
  parity bug (mili-rs kept the first); fixed in
  `mesh.rs::add_class_def`. Found by the Phase-G `mesh_object_classes`
  corpus sweep (`parity_reshape.rs`); CLAUDE.md "corpus wins".
- **`MeshObjectClass.idents_exist` is strictly TI-label-or-
  `CLASS_IDENTS`**, *not* the `NODES`/`ELEM_CONNS` id-range fallback
  `Database::labels` folds in (upstream `miliinternal.py:276-282`:
  `False` ⇔ class reaches finalisation absent a real ident source —
  e.g. sstate `cseg`). Dedicated `Database::idents_exist`; found by
  the Phase-G sweep.
- **State files carry an 8-byte per-state header** (i32 srec_id + f32
  time) — `format.md § File set`, `query.rs::state_data_start` skip.
- **VEC_ARRAY inner order is components-fastest, IP-slowest** —
  `format.md § Subrecord byte-layout matrix`.
- **`M_MESH`-superclass subrecs carry one object per state even with
  `qty_id_blks=0`** — `srec.rs::patch_m_mesh_classes`,
  `entry-payloads.md § STATE_REC_DATA`.
- **`PREC_LIMIT_DOUBLE` leaves `M_FLOAT` 4 bytes** — only explicit
  `M_FLOAT8` svars are 8 bytes. `format.md § Numeric types`.
- **Upstream `ips=` are element-set IP *labels*, not positional
  indices** — mili-python matches each `ips` value against
  `__int_points[svar][es][:-1]` via `.index(ip)` then names components
  `f"{comp} ipt. {label}"` (`miliinternal.py:1263-1270,1367`). The
  `mili-rs` core `query::Filter.ips` is 0-based *positional* into the
  vec_array inner order (`Filter.ips`, still used by the direct
  aggregate-vec_array path). **Slice B (M4-followup) landed the
  reconciliation:** `query::IntPoints` + `Database::build_int_points`
  are the `mili-rs` analogue of `__int_points`; on the
  bare-component-of-VEC_ARRAY substitution path `ips=` are interpreted
  as element-set IP *labels* and mapped to positional indices per
  subrec (`try_vec_array_substitution`). The direct
  `query("es_1a", "shell")` aggregate path keeps the positional
  `Filter.ips` (oracle-blocked, unchanged). Found by `milox` M4
  investigation; closed by the M4-followup.
- **`parallel/basic1` carries no element-set TI params (all 8
  fragments)** — unlike `serial/basic1` (`es_5`/`es_7`). Upstream
  there treats `sx`/`brick` as a plain scalar, *ignores* `ips=`, and
  does **not** raise on the cross-material query; the Slice-B oracle
  values (`3.36948112e-02`) and the must-raise contract come from
  `serial/basic1` (upstream's own `InconsistantIntPointsForElementClassResult`
  uses the serial base). The pre-encoded M4 strict-xfail pointed at
  the parallel base; repointed to serial during the M4-followup.
  Found by the Slice B corpus investigation
  (`test_query_parity.py::_BUGFIX_SLICE_B`).
- **Unfiltered query entity axis is subrecord MO ids, not labels** —
  `gather_all` emits 1-based mesh-object ordinals; the real labels
  come from the class `Labels` TI param. `query_with_labels` does the
  map (`family.rs::map_mo_ids_to_labels`, mirrors
  `miliinternal.py:1297`). Load-bearing for `DatabaseSet` merge; see
  § "Resolved: deferred query-merge parity gap".

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
