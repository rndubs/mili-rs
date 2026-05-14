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
| 10   | `query.rs` full filter set, `OBJECT_ORDERED`, vec_array     | ⏳ pending |       |                                                             |
| 11   | array-svar subscript notation (`"hx[3]"`, 1-based)          | ⏳ pending |       | `test_bugfixes.py:251-296`.                                 |
| 12   | rayon over states; criterion benches                        | ⏳ pending |       | Target ≥ 2× mili-python throughput.                         |
| 13   | cargo-fuzz on `directory.rs`, `header.rs`, `param.rs`       | ⏳ pending |       | One-hour clean-run gate.                                    |

**Phase 1 exit:** Step 8 — landed. Phase 2 (mili-py) can start now.

## Mandatory edge-case tests (per `plan.md`)

These have to pass before Phase 1 is declared done. Tracked separately
because some sit on top of multiple steps.

| Test                                          | Source                                | Status     |
|-----------------------------------------------|---------------------------------------|:-----------|
| Non-sequential mesh-object blocks coalesce    | `test_bugfixes.py:25-38`              | ✅ done (Step 5: `id_blocks: Vec<(i32, i32)>`) |
| Double-precision nodal positions              | `test_bugfixes.py:62-72`              | 🟡 metadata covered (Step 7: `dbl_nodtang`'s `nodtang` svar resolves to `NumType::Float8`); blocked on the read path itself until Step 9 |
| Vec-array with mixed component widths         | `test_bugfixes.py:119-172`            | 🟡 metadata covered (Step 7: `derive_lumps` unit test exercises mixed widths for both organisations); blocked on the read path until Step 9 |
| Inconsistent IP counts across subrecords      | `test_bugfixes.py:99-117`             | ⏳ blocked on the query layer (Step 10) — srec metadata in place |
| Array-svar subscript notation                 | `test_bugfixes.py:251-296`            | ⏳ Step 11 |
| `dir_version_2` fixture                       | corpus                                | ✅ done (Step 2) |
| State end marker `~` round-trip               | corpus (read), C oracle (write)       | ✅ read    |

## Test coverage snapshot

Snapshot at PR merge time — refresh on every step bump.

| Suite                          | Tests | Last touched |
|--------------------------------|------:|:-------------|
| `cargo test --workspace`       | 145   | Step 9       |
| Fixture parity (corpus reads)  | 42    | Step 9       |
| mili-python parity (`pyo3`)    | —     | not wired yet — Phase 2 |
| cargo-fuzz (nightly cron)      | —     | Step 13      |
| Criterion benches              | —     | Step 12      |

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

## Module shape (post-Step 9)

```
crates/mili-rs/src/
├── lib.rs              done (re-exports for Steps 0-9)
├── error.rs            done — `MiliError` (+ `Unsupported`, `NoMatchingSubrec`)
├── header.rs           done — Step 1
├── directory.rs        done — Step 2
├── param.rs            done — Step 3
├── ti.rs               done — Step 3 (v1 stub)
├── state.rs            done — Step 4
├── family.rs           done — Step 4 (Database open) + Step 9 (state-file mmap cache, `state_var_values`)
├── mesh.rs             done — Step 5
├── svar.rs             done — Step 7
├── srec.rs             done — Step 7 (includes `derive_lumps`)
├── endian.rs           done — Step 8 (`ByteSwap`, `for_each_swap`, `swap_*_slice`)
├── buffer.rs           done — Step 8 (`MiliBuffer<T>`, `pub(crate)`)
└── query.rs            done — Step 9 (RESULT_ORDERED single-svar single-state); Steps 10-11 add filters / OBJECT_ORDERED / subscript
```

## How to update this file

1. When a step lands on `main`, bump its row to ✅ done and fill in PR
   number + a one-line note about scope.
2. If the step uncovered a new edge case or open question, add a row
   under the relevant table.
3. Don't move a row to ✅ if any required test from
   `plan.md` § "Mandatory edge-case tests" is still pending — leave it
   in 🟡 partial with a pointer to the blocking test.
