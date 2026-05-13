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
| 7    | `svar.rs`, `srec.rs`, `derive_lumps`                        | ⏳ pending |       | Dual int/char stream parser; offset-math unit tests.        |
| 8    | `buffer.rs` and `endian.rs`                                 | ⏳ pending |       | Misalignment + byteswap fallbacks. Phase 1 exit criterion.  |
| 9    | `query.rs` single-svar single-state, `RESULT_ORDERED`       | ⏳ pending |       |                                                             |
| 10   | `query.rs` full filter set, `OBJECT_ORDERED`, vec_array     | ⏳ pending |       |                                                             |
| 11   | array-svar subscript notation (`"hx[3]"`, 1-based)          | ⏳ pending |       | `test_bugfixes.py:251-296`.                                 |
| 12   | rayon over states; criterion benches                        | ⏳ pending |       | Target ≥ 2× mili-python throughput.                         |
| 13   | cargo-fuzz on `directory.rs`, `header.rs`, `param.rs`       | ⏳ pending |       | One-hour clean-run gate.                                    |

**Phase 1 exit:** Step 8. Phase 2 (mili-py) can start once Step 8 lands.

## Mandatory edge-case tests (per `plan.md`)

These have to pass before Phase 1 is declared done. Tracked separately
because some sit on top of multiple steps.

| Test                                          | Source                                | Status     |
|-----------------------------------------------|---------------------------------------|:-----------|
| Non-sequential mesh-object blocks coalesce    | `test_bugfixes.py:25-38`              | ✅ done (Step 5: `id_blocks: Vec<(i32, i32)>`) |
| Double-precision nodal positions              | `test_bugfixes.py:62-72`              | ⏳ blocked on svar (Step 7) |
| Vec-array with mixed component widths         | `test_bugfixes.py:119-172`            | ⏳ blocked on svar (Step 7) |
| Inconsistent IP counts across subrecords      | `test_bugfixes.py:99-117`             | ⏳ blocked on srec (Step 7) |
| Array-svar subscript notation                 | `test_bugfixes.py:251-296`            | ⏳ Step 11 |
| `dir_version_2` fixture                       | corpus                                | ✅ done (Step 2) |
| State end marker `~` round-trip               | corpus (read), C oracle (write)       | ✅ read    |

## Test coverage snapshot

Snapshot at PR merge time — refresh on every step bump.

| Suite                          | Tests | Last touched |
|--------------------------------|------:|:-------------|
| `cargo test --workspace`       | 78    | Step 6       |
| Fixture parity (corpus reads)  | 23    | Step 6       |
| mili-python parity (`pyo3`)    | —     | not wired yet — planned for Step 8 |
| cargo-fuzz (nightly cron)      | —     | Step 13      |
| Criterion benches              | —     | Step 12      |

## Resolved questions log

Track in `plan.md` § "Resolved questions". Current entries:

- `PREC_LIMIT_DOUBLE` semantics — pre-Step 1.
- Format-v1 directory support — deferred with typed `UnsupportedDir(1)`.
- Label / material trailing convention — labels and elem-ids are
  separate TI arrays of equal length, not split halves.

## Open questions (still active)

Surfaced in `plan.md` § "Open questions to revisit during implementation":

- `block_obj_fmt` connectivity prevalence — defer code path until a
  fixture trips it.
- mmap on Lustre / NFS — defer the pread fallback until a benchmark
  motivates it.
- UTF-8 strictness — currently strict; downgrade to lossy behind a
  feature flag if a real fixture breaks.
- `MiliBuffer` public-vs-private — keep `pub(crate)` until Step 8 has
  a concrete need from `mili-py` / `mili-viz`.
- Element-set name → material id parse rule — Step 6 currently maps
  only sets whose name parses as `i32` to `integration_points`,
  matching the simplest reading of `miliinternal.py:463-474`. Revisit
  once a fixture surfaces a non-integer setname.

## Module shape (post-Step 6)

```
crates/mili-rs/src/
├── lib.rs              done (re-exports for Steps 0-5)
├── error.rs            done — `MiliError`
├── header.rs           done — Step 1
├── directory.rs        done — Step 2
├── param.rs            done — Step 3
├── ti.rs               done — Step 3 (v1 stub)
├── state.rs            done — Step 4
├── family.rs           done — Step 4 (Database open)
├── mesh.rs             done — Step 5
├── svar.rs             todo — Step 7
├── srec.rs             todo — Step 7
├── buffer.rs           todo — Step 8
├── endian.rs           todo — Step 8
└── query.rs            todo — Steps 9-11
```

## How to update this file

1. When a step lands on `main`, bump its row to ✅ done and fill in PR
   number + a one-line note about scope.
2. If the step uncovered a new edge case or open question, add a row
   under the relevant table.
3. Don't move a row to ✅ if any required test from
   `plan.md` § "Mandatory edge-case tests" is still pending — leave it
   in 🟡 partial with a pointer to the blocking test.
