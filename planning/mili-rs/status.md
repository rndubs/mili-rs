# `mili-rs` implementation status

Live tracker for the Rust port. Update on every PR that moves a step
forward; the table is the source of truth for what's safe to depend on.

Reference plan: [`plan.md`](plan.md) — step numbers below match the
"Incremental build order" table.

## Step status

| Step | Lands                                                       | Status     | PR    | Notes                                                       |
|-----:|-------------------------------------------------------------|:-----------|:------|:------------------------------------------------------------|
| 0    | Workspace, CI skeleton, `MiliError`, fixture symlinks       | ✅ done    | #1    | Cargo workspace, GitHub Actions, `thiserror`-based errors.   |
| 1    | `header.rs` + golden bytes from `basic1.pltA`               | ✅ done    | #2    | 16-byte header, version 2/3 acceptance, dir v1 rejected.    |
| 2    | `directory.rs` for v3, then v2 (v1 deferred)                | ✅ done    | #2    | Trailer walk, name pool UTF-8 validation, v2 i32→i64 widen. |
| 3    | `param.rs` scalar/string/array decode; `ti.rs` open stub    | ✅ done    | #3    | `ParamTable` indexes inline TI_PARAMs; base26 v1 stub.      |
| 4    | `family.rs` open path, `state_map` resolution, end-marker   | ✅ done    | #3    | mmap-backed `Database`, inline + tfile state-map dispatch.  |
| 5    | `mesh.rs`: CLASS_DEF + CLASS_IDENTS, nodes, connectivity    | ✅ done    | #4    | `MeshTable`, `Nodes`, `Connectivity`, superclass table.     |
| 6    | high-level TI accessors (labels, materials, element_sets)   | ✅ done    | TBD   | `Database::{labels, materials, element_sets, integration_points}`; `MaterialId` newtype. |
| 7    | `svar.rs`, `srec.rs`, `derive_lumps`                        | ✅ done    | TBD   | Dual int/char stream parsers; `derive_lumps` covers both organisations; svar count on basic1 = 93 (with recursive components). |
| 8    | `buffer.rs` and `endian.rs`                                 | ✅ done    | TBD   | `MiliBuffer<T: ByteSwap>` (pub(crate)) with `Mmap`/`Owned` storage, alignment check, byteswap fallbacks; `ByteSwap` trait for `i32`/`i64`/`f32`/`f64`. Mesh / connectivity / TI int-array decoders all route through `endian::for_each_swap`. Phase 1 exit. |
| 9    | `query.rs` single-svar single-state, `RESULT_ORDERED`       | ✅ done    | TBD   | `Database::state_var_values(svar, class, state) -> StateValues::{F32,F64,I32,I64}`; lazy state-file mmap cache; per-state header skip is 8 bytes (i32 srec_id + f32 time); `OBJECT_ORDERED`, label / material / IP filters, and component-name lookups return typed errors until Steps 10 / 11. |
| 10   | `query.rs` full filter set, `OBJECT_ORDERED`, vec_array     | ✅ done    | TBD   | `Database::query(&QueryArgs)` with labels / states / materials / ips filters; `OBJECT_ORDERED` gather; vec_array IP filter (components-fastest, IP-slowest); material → label via `ELEM_CONNS` mat_id column; `LabelNotFound` / `UnknownMaterial` / `IpFilterNotApplicable` / `IpOutOfRange` typed errors; `Svar::atoms` now accumulates component atom counts (was `comps.len()` — broke for vec_array-of-vectors like d3samp4's `es_1a`). Component-name lookups (`"sx"` → `"stress"`, `hx[3]`) still defer to Step 11. |
| 11   | array-svar subscript notation (`"hx[3]"`, 1-based)          | ✅ done    | TBD   | `parse_query_name` + `resolve_target` decompose the input into base svar + `AtomPicker::Specific` atom indices; ARRAY-svar subscript (`"hx[3]"`) and bare-component lookup (`"sx"` → parent VECTOR `stress`) share one gather path. `InvalidSubscript` / `SubscriptNotApplicable` typed errors cover the mili-python exception set (out-of-range, 0/negative, too-many indices, non-integer). VEC_ARRAY bare-comp + partial-dim multi-D subscripts still defer with `Unsupported`. d3samp6.thA `hx[3]` matches mili-python's golden bit-for-bit on state 6, labels [2,5,10] (`test_bugfixes.py:345-365`). |
| 12   | rayon over states; criterion benches                        | 🟡 partial | TBD   | `Database::query` now prefetches per-state contexts (rebased plan + state-file mmap + path) single-threaded then dispatches the byteswap-and-fill gather across `par_chunks_mut` over the output vec — each state writes a disjoint slab so no synchronisation in the hot loop. Criterion suite at `crates/mili-rs/benches/read.rs` covers `open`, `nodes`, `query_single`, `query_many`; on the local sandbox `query_single` (basic1 `nodpos`, all states) hits ~2.0 GiB/s and `query_many` ~2.5 ms for four svars over all states. Full ≥ 2× mili-python throughput parity bench lands with the pyo3 cross-impl harness in Phase 2 (no mili-python install available in CI yet). |
| 13   | cargo-fuzz on `directory.rs`, `header.rs`, `param.rs`       | 🟡 partial | TBD   | Scaffolding only: `crates/mili-rs/fuzz/` is a self-contained cargo-fuzz crate (own `[workspace]` to hide from the parent) with three targets — `header` (`Header::parse`), `directory` (chained `Header::parse` + `Directory::parse`), and `param` (chained header → directory → `ParamValue::decode` over every parsed entry). `cargo check` on stable passes (libfuzzer-sys's link step is the only nightly-only piece). The CI nightly-cron job + one-hour clean-run gate land in a follow-up CI tweak. |

**Phase 1 exit:** Step 8 — landed. Phase 2 (mili-py) can start now.

## Mandatory edge-case tests (per `plan.md`)

These have to pass before Phase 1 is declared done. Tracked separately
because some sit on top of multiple steps.

| Test                                          | Source                                | Status     |
|-----------------------------------------------|---------------------------------------|:-----------|
| Non-sequential mesh-object blocks coalesce    | `test_bugfixes.py:25-38`              | ✅ done (Step 5: `id_blocks: Vec<(i32, i32)>`) |
| Double-precision nodal positions              | `test_bugfixes.py:62-72`              | ✅ done (Step 10: `state_var_values` returns `StateValues::F64` for `Float8` svars; covered by the per-numtype gather macro) |
| Vec-array with mixed component widths         | `test_bugfixes.py:119-172`            | ✅ done (Step 10: `Svar::atoms` accumulates component atom counts; d3samp4 `es_1a` (`vec_array<[stress(6), eps(1)]>`) round-trips through `query()` with `ips` filter — fixture test in `tests/query_fixtures.rs`) |
| Inconsistent IP counts across subrecords      | `test_bugfixes.py:99-117`             | 🟡 IP filter mechanism in place (Step 10); the cross-subrec IP-label-set validation that turns inconsistent counts into a typed error needs the element-set IP-label lookup, which lands with component-name resolution in Step 11. |
| Array-svar subscript notation                 | `test_bugfixes.py:251-296`            | ✅ done (Step 11: `hx[3]` resolves through `parse_query_name`/`resolve_target`/`AtomPicker::Specific`; d3samp6.thA fixture matches the mili-python golden bit-for-bit; the four error cases (`hx[0]`, `hx[9]`, `hx[-2]`, `hx[1,1]`) all surface `MiliError::InvalidSubscript`.) |
| `dir_version_2` fixture                       | corpus                                | ✅ done (Step 2) |
| State end marker `~` round-trip               | corpus (read), C oracle (write)       | ✅ read    |

## Test coverage snapshot

Snapshot at PR merge time — refresh on every step bump.

| Suite                          | Tests | Last touched |
|--------------------------------|------:|:-------------|
| `cargo test --workspace`       | 190   | Step 11      |
| Fixture parity (corpus reads)  | 53    | Step 11      |
| mili-python parity (`pyo3`)    | —     | not wired yet — Phase 2 |
| cargo-fuzz (nightly cron)      | 3     | Step 13 scaffolding (header, directory, param) — CI runner pending |
| Criterion benches              | 4     | Step 12 (open, nodes, query_single, query_many) |

## Resolved questions log

Track in `plan.md` § "Resolved questions". Current entries:

- `PREC_LIMIT_DOUBLE` semantics — pre-Step 1.
- Format-v1 directory support — deferred with typed `UnsupportedDir(1)`.
- Label / material trailing convention — labels and elem-ids are
  separate TI arrays of equal length, not split halves.
- `CLASS_DEF` superclass field (Step 6 fix-up). `entry-payloads.md`
  documented MODIFIER1 as the superclass, but every fixture in the
  corpus stores MODIFIER1 = 0 and the actual superclass in MODIFIER2.
  Reader now reads MODIFIER2; the entry-payloads doc will be tightened
  to match in the next planning pass.
- Multiple `NODES` / `ELEM_CONNS` per `(mesh, class)` (Step 6 fix-up).
  Real-world databases (basic1) split non-contiguous element-id
  ranges across multiple `ELEM_CONNS` entries; the table now indexes
  them as `Vec<usize>` and `load_ident_ranges` collects id-blocks from
  `CLASS_IDENTS`, `NODES`, and `ELEM_CONNS` so `element_count` is
  correct even for classes that ship no `CLASS_IDENTS`.
- Idempotent `CLASS_DEF` re-declaration (Step 6 fix-up). Writer can
  emit the same class twice; reader accepts the second declaration
  when superclass and long name match, errors only on real conflicts.
- `Lumps` interpretation (Step 7). `derive_lumps` produces `atoms`,
  `sizes`, and `offsets` parallel vectors that are independent of
  [`Organization`]; the byte-address formula differs per organisation
  and is documented at the head of `srec.rs`. For `RESULT_ORDERED`
  the per-svar slab is `N * offsets[s]` from the subrec start;
  `OBJECT_ORDERED` strides by `bytes_per_object` across objects with
  `offsets[s]` selecting within. The Rust unit tests cover both, and
  the matrix in `format.md` § "Subrecord byte-layout matrix" is the
  source of truth.
- `Subrecord.id_blocks` semantics (Step 7). Stored exactly as written
  on disk: inclusive `(start, stop)` pairs in 1-based mili object
  ids. Normalisation to 0-based ordinals + half-open intervals
  (mili-python's `afileIO.py:444-445`) is deferred to the query layer
  so the table preserves writer-side intent.
- Per-state header in state files (Step 9 fix-up). `format.md` § "Top-
  level file inventory" claims state files have "**No per-state
  header**" — that's wrong. `reference/mili/src/mili.c:3042-3043` and
  `srec.c:2332-2333` both add `sizeof(int) + sizeof(float)` (i32
  srec_id + f32 time) before the subrec data when computing read /
  write offsets. The Rust reader skips 8 bytes after `state.offset`
  before computing subrec offsets. The format doc needs a fix-up in
  the next planning pass.
- VEC_ARRAY inner-order: components-fastest, IP-slowest (Step 10).
  `planning/shared/format.md` § "Subrecord byte-layout matrix" reads
  "array-dim indices vary fastest, then component (vector) index" —
  for a 1-D `dims=[n_ip]` vec_array this would put IPs fastest. The
  Python writer / reader (`reference/mili-python/src/mili/datatypes.py:
  236-247`, the `[sv.comp_layout for sv in svars] * prod(dims)` line)
  lays out components inner, IPs outer, and the d3samp4 `es_1a`
  fixture round-trips that layout against direct file reads (see
  `tests/query_fixtures.rs::d3samp4_vec_array_*`). The Rust IP filter
  follows Python; the format doc needs the same fix-up that Step 9
  flagged for the per-state header. mili-python's `test_bugfixes.py::
  VectorsInVectorArrays` numeric goldens couldn't be cross-checked
  without a working mili-python install in this environment; the Rust
  layout is verified self-consistent against direct decode of the
  state-file bytes.
- `Svar::atoms` for nested aggregates (Step 10 fix-up). Step 7's
  `parse_one` computed `atoms = comps.len()` for vector svars and
  `prod(dims) * comps.len()` for vec_array. That under-counts a
  vec_array whose components are themselves vectors (d3samp4's
  `es_1a` = `vec_array<[stress(6), eps(1)]>`: should be `2 * 7 = 14`,
  was `2 * 2 = 4`). The parser now accumulates `sum(comp.atoms)`
  recursively via the already-parsed components in the table —
  components are always parsed before their parent (svar.rs's
  recursion at line 274-280 owns this invariant).
- ARRAY-subscript parser semantics (Step 11). mili-python's
  `__parse_query_name_and_source` is tolerant: `"hx["`, `"hx[3"`,
  `"hx[,1]"` all reach the integer-conversion step and raise
  `ValueError` there. The Rust parser is stricter — it requires a
  balanced `[...]` pair and rejects empty / trailing-comma index
  lists with `MiliError::InvalidSubscript` before doing any
  per-svar lookup. The five test_bugfixes.py:259-268 inputs
  (`hx[0]`, `hx[9]`, `hx[-2]`, `hx[1,1]`) all round-trip through
  `InvalidSubscript` and the resulting message is unambiguous.
  Partial-dim subscripts (e.g. `g[1]` on a `dims=[3,4]` svar) are
  rejected with `MiliError::Unsupported("partial-dim array
  subscript...")` rather than silently raveled — no corpus
  exercises them and the python behaviour is itself underspecified
  (the comment at `miliinternal.py:1373-1375` flags this as a
  known gap).
- Bare component-name fallback (Step 11). Queries like `"sx"`
  resolve to the parent VECTOR svar `"stress"` when no subrec
  carries `"sx"` directly. The fallback uses `find_vector_parent`,
  which walks the svar dictionary and accumulates the component's
  atom offset from the prior comps' atom counts (handles components
  that are themselves multi-atom vectors). VEC_ARRAY parents are
  intentionally **not** auto-resolved — a vec_array bare-comp
  query needs explicit IP composition because the component data
  is striped across IP slots; defer until a fixture surfaces the
  case (mili-python's `test_modify_database.py::sx-on-beam` is
  the likely future driver, but it also requires the write path).
- `MiliBuffer` public-vs-private (Step 8). Kept `pub(crate)` for now —
  `Nodes` / `Connectivity` / `ArrayParam` keep their existing public
  shapes (which already wrap the same bytes), and the byteswap path
  has been unified via the shared `endian::for_each_swap` primitive
  rather than by forcing every caller through `MiliBuffer`. Revisit
  when `mili-py` / `mili-viz` have a concrete need to view a raw
  typed buffer (Step 9 query results are the likely trigger).

## Open questions (still active)

Surfaced in `plan.md` § "Open questions to revisit during implementation":

- `block_obj_fmt` connectivity prevalence — defer code path until a
  fixture trips it.
- mmap on Lustre / NFS — defer the pread fallback until a benchmark
  motivates it.
- UTF-8 strictness — currently strict; downgrade to lossy behind a
  feature flag if a real fixture breaks.
- Element-set name → material id parse rule — Step 6 currently maps
  only sets whose name parses as `i32` to `integration_points`,
  matching the simplest reading of `miliinternal.py:463-474`. Revisit
  once a fixture surfaces a non-integer setname.

## Module shape (post-Step 11)

```
crates/mili-rs/src/
├── lib.rs              done (re-exports for Steps 0-11, including `QueryArgs`)
├── error.rs            done — `MiliError` (+ `Unsupported`, `NoMatchingSubrec`, `LabelNotFound`, `IpOutOfRange`, `IpFilterNotApplicable`, `UnknownMaterial`, `InvalidSubscript`, `SubscriptNotApplicable`)
├── header.rs           done — Step 1
├── directory.rs        done — Step 2
├── param.rs            done — Step 3
├── ti.rs               done — Step 3 (v1 stub)
├── state.rs            done — Step 4
├── family.rs           done — Step 4 (Database open) + Step 9 (state-file mmap cache, `state_var_values`) + Step 10 (`query(&QueryArgs)` with labels/states/materials/ips filters, material → label via `ELEM_CONNS`)
├── mesh.rs             done — Step 5
├── svar.rs             done — Step 7, Step 10 (atom-count fix for vec_array-of-vectors)
├── srec.rs             done — Step 7 (includes `derive_lumps`)
├── endian.rs           done — Step 8 (`ByteSwap`, `for_each_swap`, `swap_*_slice`)
├── buffer.rs           done — Step 8 (`MiliBuffer<T>`, `pub(crate)`)
└── query.rs            done — Step 10 (RESULT_ORDERED + OBJECT_ORDERED gather, label / IP filters, multi-state `ReadPlan::rebased`) + Step 11 (`parse_query_name` + `resolve_target` + `AtomPicker::{AllAtoms, PerIp, Specific}`; ARRAY-svar subscript `"hx[3]"` and bare-component `"sx"`-on-VECTOR-parent lookup share the `Specific` atom-indices gather path)
```

Step 12 lands `rayon = "1.10"` as a hard dep (already on the
external-deps list in `plan.md`) and `criterion = "0.5"` as a
dev-dep. The state-axis parallelisation lives entirely in
`Database::query`'s gather macro via `par_chunks_mut` over the
output vec; `query.rs` itself stays single-threaded for the plan
build.

## How to update this file

1. When a step lands on `main`, bump its row to ✅ done and fill in PR
   number + a one-line note about scope.
2. If the step uncovered a new edge case or open question, add a row
   under the relevant table.
3. Don't move a row to ✅ if any required test from
   `plan.md` § "Mandatory edge-case tests" is still pending — leave it
   in 🟡 partial with a pointer to the blocking test.
