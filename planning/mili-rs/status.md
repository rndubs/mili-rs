# `mili-rs` implementation status

Live tracker. **Source of truth for what's safe to depend on**; the
design rationale lives in [`plan.md`](plan.md).

## What's next

**Phase 1 is complete** modulo a single CI-time gate. The natural next
move is **starting Phase 2: `mili-py`** — see
[`../mili-py/README.md`](../mili-py/README.md) for the existing design
notes and the M1→M6 milestone breakdown. The Rust-side surface is
narrowed and validated; Phase 2 wraps `Database::open` /
`Database::query` and replaces the existing pure-Python `mili` package.

The one Phase-1 follow-up that can't be closed from a normal PR:

- **Step 13 fuzz cron clean-run gate.** CI job exists
  (`.github/workflows/ci.yml § fuzz`, scheduled `0 7 * * *` UTC). Flips
  to ✅ once a maintainer confirms the first cron run lands clean (or
  manually triggers `workflow_dispatch` from the Actions UI). No code
  change required from here.

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

## Test coverage snapshot

| Suite                          | Tests | Notes |
|--------------------------------|------:|:------|
| `cargo test --workspace`       | 191   | unit + fixture integration |
| mili-python parity (`pyo3`)    | 17    | 12 corpus fixtures bit-exact; `cargo test --features parity` |
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
the bindings layer should surface the typed errors cleanly.

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

## How to update this file

1. When a Phase-2 step lands, add a row to the appropriate section.
   Don't re-open closed Phase-1 rows.
2. If a closed step regresses, demote it to 🟡 with the failing test
   linked — don't paper over with notes.
3. New surprises go under "Surprises worth remembering" with a one-line
   pointer and a code reference. Keep retrospectives in the commit
   message, not here.
