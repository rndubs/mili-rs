# `mili-rs` implementation plan

> **Status (Phase 1 complete):** Steps 0–12 + 14–16 ✅; Step 13 🟡
> (cron clean-run gate pending). The live tracker is
> [`status.md`](status.md). This document is the **design archive** —
> it captures the build order, oracle strategy, and CI shape that got
> us here. Don't edit it to track progress; edit it only if the
> *design* changes (e.g. Phase 2 reshapes a module boundary).

The README in this directory states the goals and high-level
milestones. This document is the working plan: concrete modules,
build order, oracle strategy, CI shape.

`mili-rs` has no upstream dependency on the other layers. `mili-py`
and `mili-viz` consume `mili-rs`; the shared `MiliBuffer<T>` contract
(`../shared/buffer.md`) is something `mili-rs` defines first and the
others import. There are no cross-layer uncertainties to unblock
before starting.

## Workspace bootstrap

Initial repo layout after Phase 1 starts:

```
Cargo.toml              # [workspace], members = ["crates/mili-rs"]
rust-toolchain.toml     # pinned stable + components
.github/workflows/ci.yml
crates/mili-rs/
├── Cargo.toml
├── build.rs            # used only to feature-gate mmap on platforms that need it
├── src/
│   └── lib.rs
├── tests/
│   ├── fixtures/       # symlink or copy of a small subset of reference/mili-python/tests/data
│   ├── parity_*.rs     # parity tests vs mili-python
│   └── round_trip_*.rs # byte-level diff vs C oracle (Phase 3 onward)
├── benches/
│   └── read.rs         # criterion
└── fuzz/               # cargo-fuzz targets, added at M6
```

Dependencies in `crates/mili-rs/Cargo.toml`:

- `memmap2` — file-backed buffers.
- `bytemuck` — `Pod`/`Zeroable` for typed views over `&[u8]`.
- `byteorder` — endianness conversions on copy paths.
- `rayon` — opportunistic parallelism (M6 onward).
- `thiserror` — `MiliError`.
- `ndarray` — public result shape (`Array3<T>`).
- `hashbrown` (or `std::collections::HashMap`) — internal lookups.
- `tracing` — structured logs, off by default.

Dev-dependencies: `criterion`, `proptest`, `tempfile`, `pyo3` (test-only,
for invoking mili-python as an oracle in integration tests).

## Module-by-module plan

Each module corresponds to a Rust source file under
`crates/mili-rs/src/`. Internal modules are private; only the
re-exports listed in the README form the public API.

### `error.rs`

Single `MiliError` enum, `thiserror`-derived:

```rust
#[derive(thiserror::Error, Debug)]
pub enum MiliError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("bad magic: expected 'mili', got {0:?}")] BadMagic([u8; 4]),
    #[error("unsupported header version {0}")]      UnsupportedHeader(u8),
    #[error("unsupported directory version {0}")]   UnsupportedDir(u8),
    #[error("truncated {file}: needed {need} bytes at offset {off}, got {got}")]
        Truncated { file: PathBuf, off: u64, need: usize, got: usize },
    #[error("directory entry {idx} points past EOF (offset {off}, len {len}, file size {size})")]
        DirEntryOutOfRange { idx: usize, off: u64, len: u64, size: u64 },
    #[error("bad UTF-8 in name pool at offset {0}")]  BadName(usize),
    #[error("unknown svar {0:?}")]                    UnknownSvar(String),
    #[error("unknown class {0:?}")]                   UnknownClass(String),
    #[error("state {0} out of range (0..{1})")]       StateOutOfRange(usize, usize),
    #[error("misaligned mmap: offset {0} not aligned for {1}-byte type")]
        Misaligned(usize, usize),
}
```

The reader mirrors the C library's defensive-on-I/O / permissive-on-
content posture, but we **do** validate magic and version bytes
(matching the C lib here would silently accept garbage; we don't).

### `header.rs`

The 16-byte char header. Pure parsing, no I/O. Returns a `Header`
struct with: header version, dir version, endianness, precision
limit, suffix width, partition scheme byte, extension count. The
endianness flag drives every numeric read downstream.

### `directory.rs`

Trailer-based parser. Steps:

1. Stat the `.A` file to learn its size.
2. Seek `-(QTY_DIR_HEADER_FIELDS * 4)` from EOF, read the 4-int
   directory header.
3. From `QTY_ENTRIES` plus the dir version, compute total entry
   bytes (v1/v2: `QTY * 6 * 4`; v3: `QTY * 6 * 8`). Seek backward
   again and read entries.
4. Seek backward by `NAMES_LEN` and read the name pool. Validate
   UTF-8 once; index it into a `Vec<&str>`.
5. v1/v2 entries: widen 4-byte ints to `i64` in memory so downstream
   code only sees one entry shape.

Output: `Directory { entries: Vec<DirEntry>, names: NamePool }`.

### `family.rs`

`Database::open(path)`:

1. Open and mmap `R.A`.
2. Parse header + directory.
3. Resolve scalar params for partition scheme and per-file limits
   (`"states per file"`, `"max size per file"`).
4. Read the `state_map` (file index, byte offset, time, srec format
   id per state). For header v3+ databases with a `.tfile`, pull
   the state count from there instead of the directory header.
5. Lazy-open state files (`R.A00`, `R.A01`, …) on first access; mmap
   them and stash the `Arc<Mmap>` in a `Vec<Option<Arc<Mmap>>>`.
6. Lazy-open TI files (`R.ATI0`, …) the same way, with their own
   directory parsed identically.

The `Database` is `Send + Sync` once construction completes. All
post-open access is through immutable references to the directory
and `Arc`-cloned mmap handles, so multi-threaded reads are safe by
construction — no global `fam_list` equivalent.

### `mesh.rs`

Materializes mesh metadata from the directory:

- `Mesh` (one per mesh discovered while scanning entries; the mesh
  id-space is the union of all `MODIFIER1` values on geometry
  entries).
- `ObjectClass` (one per `CLASS_DEF` entry): name, superclass code,
  long name. The element count and id range come from one or more
  `CLASS_IDENTS` entries for the same `(mesh_id, classname)` —
  multiple CLASS_IDENTS entries per class are coalesced into a
  block list (`Vec<(i32, i32)>`).
- `nodes(MeshId) -> MiliBuffer<f32>`: locate the `NODES` entry,
  skip the `[start, stop]` int prefix, return a buffer over the
  coordinate range. `fam.dimensions` is read from the
  `"mesh dimensions"` scalar param at open and lives on `Database`.
- `connectivity(ClassId) -> MiliBuffer<i32>`: parse the
  `[superclass, qty_blocks][block_pairs...]` header, then expose
  the connectivity data. The block list itself is preserved on
  `ObjectClass` for label / material lookup against TI params.
- The labels-per-class accessor lives in `param.rs` (see below) and
  is exposed re-exported through `mesh.rs` for ergonomics.

### `svar.rs`

Parses `STATE_VAR_DICT` entries into typed `Svar` records: name,
title, `num_type`, `agg_type`, rank, dims, component names.

The dictionary's payload is a dual stream (`entry-payloads.md` §
`STATE_VAR_DICT`): a Rust parser walks the integer and character
streams in lockstep, consuming one svar definition at a time.
Components of vector / vec_array svars are NUL-terminated names
embedded **inside** the char stream, not in the file-level name
pool — the parser owns its own substring index into the char stream
and does not touch the global name pool for components.

The resulting `Svar` carries:

```rust
struct Svar {
    name: String,
    title: String,
    num_type: NumType,    // M_INT4 / M_INT8 / M_FLOAT4 / M_FLOAT8
    agg: SvarAgg,         // Scalar | Vector { comps: Vec<String> } | Array { dims: Vec<i32> } | VecArray { dims: Vec<i32>, comps: Vec<String> }
    atoms: usize,         // resolved at parse time per the matrix in format.md
}
```

A user query for `"sx"` against a vector svar `"stress"` resolves
by matching `"sx"` against `stress.agg.comps` and recording the
component index for the byte-offset math in `query.rs`.

### `srec.rs`

State record format parser. One `Srec` per srec id; each holds a
`Vec<Subrecord>` with:

- `mclass`: resolved class id (from looking up the on-disk class
  name via the class table).
- `svar_ids`: resolved svar ids (from looking up names via the
  svar table).
- `organization`: `RESULT_ORDERED` or `OBJECT_ORDERED`.
- `id_blocks`: `Vec<(i32, i32)>` of inclusive object id ranges
  (potentially the whole class, potentially a subset).
- Derived per-svar `lump_offsets`, `lump_sizes`, `lump_atoms` —
  computed on load via the algorithm in `srec.c:1409+`, *not*
  read from disk. The Rust function `derive_lumps` lives here and
  is unit-tested against handcrafted inputs first, against a real
  fixture second.

The byte-layout matrix (`format.md` § "Subrecord byte-layout
matrix") drives the offset arithmetic in `query.rs`; `srec.rs` is
its source of truth.

### `param.rs`

Parses MILI_PARAM, APPLICATION_PARAM, and TI_PARAM payloads. One
parser, three call sites (the main `.A` directory and each `R.ATI*`
directory).

Public API (called from `family.rs` and re-exported by
`Database`):

- `scalar<T>(&self, name: &str) -> Result<T>` for the basic
  partition-limit / dimension reads.
- `string(&self, name: &str) -> Result<&str>`.
- `array<T>(&self, name: &str) -> Result<ArrayParam<T>>` returning
  `{ dims: Vec<i32>, data: MiliBuffer<T> }`.

On top of these, **high-level accessors** that follow the
TI_PARAM naming conventions documented in `format.md`:

- `labels(class: ClassId) -> MiliBuffer<i32>` → reads
  `Labels[<classname>]`.
- `element_ids(class: ClassId) -> MiliBuffer<i32>` → reads
  `Labels-ElemIds[<classname>]`.
- `materials() -> HashMap<String, Vec<i32>>` → scans for
  `MAT_NAME_<n>` TI params.
- `element_sets() -> HashMap<String, ElementSet>` → scans for
  `IntLabel_es_<name>`; the trailing entry is the count and the
  preceding entries are integration-point ids, per the contract
  enforced in `miliinternal.py:113-115`.
- `integration_points() -> HashMap<MaterialId, Vec<i32>>` →
  derived from element_sets per `miliinternal.py:463-474`.

These accessors are **the** way to discover the high-level
concepts the Python API exposes. There is no parallel dedicated
entry-type code path.

### `state.rs`

Per-state location and timing. `StateMeta { file_idx, offset, time,
srec_format }`. Plus a helper `state_bytes(idx) -> &[u8]` that uses
the next state's offset (or EOF) to compute length and slice into
the mmap of the right state file.

### `query.rs`

The hot path. Inputs:

```rust
pub struct Query<'a> {
    pub svars: &'a [&'a str],
    pub class: &'a str,
    pub labels: Option<&'a [i32]>, // None means "all"
    pub states: Option<&'a [usize]>,
    pub materials: Option<&'a [i32]>,
    pub ips: Option<&'a [usize]>,
}
```

Output: a per-svar `Array3<T>` indexed `(state, entity, component)`
plus a `Layout` struct (selected labels/states/components/times) —
shape-compatible with mili-python's `QueryDict`.

Pipeline:

1. **Resolve.** Map svar names to svar ids; map class to class id;
   resolve label filter → ordinals; resolve state filter →
   `Vec<usize>`.
2. **Plan.** For each (state, subrecord, svar), compute source byte
   ranges and destination strides in the output `Array3`. This is
   pure metadata work and is fully precomputable from immutable
   state.
3. **Execute.** Walk the plan filling the pre-allocated output.
   `RESULT_ORDERED` contiguous svar+state → one `copy_from_slice`
   per state (or a single typed view when alignment + endianness
   permit no-copy). `OBJECT_ORDERED` → strided gather.
4. **Parallelize (M6).** The plan is partitioned along the state
   axis, run under `rayon`. The output `Array3` is split into
   non-overlapping slabs by state index, so threads write disjoint
   memory.

The pre-allocated output is the cure for mili-python's `np.concatenate`
hot path (`reference/mili-python/src/mili/miliinternal.py:1414-1416`).

### `buffer.rs`

Implementation of `MiliBuffer<T>`. Carries an `Arc<Storage>` plus
offset/length/byteswap state. `as_slice() -> Option<&[T]>` returns
`Some` iff aligned and native-endian; otherwise callers use
`to_owned()`. See `../shared/buffer.md` for the contract.

### `endian.rs`

Byteswap helpers for the four numeric widths we care about (`i32`,
`i64`, `f32`, `f64`). Used on the copy-fallback paths in `buffer.rs`
and the query gather code.

### `ti.rs`

TI file open / directory load. Same directory parser as
`directory.rs` but pointed at the separate `R.ATI0…` files. Holds
its own `Directory` and name pool. Hands TI_PARAM entries off to
`param.rs` for payload parsing — there is exactly one param parser
and it is reused here.

### `state_marker.rs` (small)

Read and write of the `~` (0x7E) end-of-file marker in the
`.tfile`. Read path validates and returns a typed error on
mismatch; write path emits a single byte after the state map is
flushed. Trivial but called out because forgetting it on write
will silently produce files the C reader thinks are corrupt.

## Incremental build order

The order below is what we land on the branch in sequence. Each step
is a PR-sized chunk that compiles, ships tests, and doesn't regress
prior work.

| Step | Lands                                                       | Validates against                                                 | Status |
|-----:|-------------------------------------------------------------|-------------------------------------------------------------------|:-------|
| 0    | Workspace, CI skeleton, `MiliError`, fixture symlinks       | `cargo test` green                                                | ✅ |
| 1    | `header.rs` + golden bytes from `basic1.pltA`               | hand-checked offsets, precision-limit resolution                  | ✅ |
| 2    | `directory.rs` for v3, then v2 (v1 deferred)                | parity on dir-entry count, names, types; `dir_version_2/` fixture | ✅ |
| 3    | `param.rs` scalar/string/array decode; `ti.rs` open         | parity on `"mesh dimensions"`, `"states per file"`, MAT_NAME, IntLabel_es | ✅ |
| 4    | `family.rs` open path, `state_map` resolution, end-marker   | parity on `times()`, state count, marker round-trip               | ✅ |
| 5    | `mesh.rs`: CLASS_DEF + CLASS_IDENTS, nodes, connectivity    | parity on `class_names()`, `nodes()`, `connectivity()`            | ✅ |
| 6    | high-level TI accessors (labels, materials, element_sets)   | parity on `labels()`, `materials()`, `element_sets()`, `integration_points()` | ✅ |
| 7    | `svar.rs`, `srec.rs`, `derive_lumps`                        | parity on svar metadata; unit tests on offset math for every cell of the layout matrix | ✅ |
| 8    | `buffer.rs` and `endian.rs` — **Phase 1 exit**              | unit tests on synthetic mmaps, including misaligned + byteswap cases | ✅ |
| 9    | `query.rs` single-svar single-state, `RESULT_ORDERED`       | parity on simple `query()` cases                                  | ✅ |
| 10   | `query.rs` full filter set, `OBJECT_ORDERED`, vec_array     | full mili-python read test suite                                  | ✅ |
| 11   | array-svar subscript notation (`"hx[3]"`, 1-based)          | parity on `test_bugfixes.py:251-296`                              | ✅ |
| 12   | rayon over states; criterion benches                        | ≥ 2× mili-python throughput                                       | ✅ |
| 13   | cargo-fuzz targets on `directory.rs`, `header.rs`, `param.rs` | runs clean for an hour (cron-time gate)                         | 🟡 |
| 14   | pyo3 cross-impl parity harness                              | post-plan: bit-exact `db.query()` round-trip                      | ✅ |
| 15   | nightly fuzz CI cron + planning-doc fix-ups                 | CI workflow; `format.md`/`entry-payloads.md` corrections          | ✅ |
| 16   | Phase-1 closeout: corpus-wide parity, IP-count contract, API audit | parity across 12 fixtures; `#[doc(hidden)]` narrowing      | ✅ |

Step 0 lands a working CI before any logic. Steps 1–6 land
read-side metadata; nothing returns data arrays yet. Step 7 is the
first end-to-end "user can read a result" milestone. Step 8 is the
Phase 1 exit criterion (full read parity). Steps 9–11 harden.

Steps 14–16 were added after the original plan as the pyo3 parity
harness surfaced corpus-wide validation needs; the live status of
each is in [`status.md`](status.md).

The write path (Phase 3) gets its own plan document later, but the
modules it touches (`directory.rs`, `srec.rs`, `state.rs`) already
need to be structured with write support in mind — keep parser
state inspectable, avoid baking read-only assumptions into types.

## Compatibility testing strategy

Three layers of validation, each catching a different failure class.

### Layer 1: parity vs. mili-python (read-side oracle)

mili-python has no pre-baked `.npy` fixtures; all assertions go
through its `MiliDatabase` API (`reference/mili-python/tests/`). So
we use mili-python *itself* as the oracle, invoked from Rust
integration tests via `pyo3`:

```rust
// crates/mili-rs/tests/parity_basic1.rs
fn oracle(py: Python<'_>, fixture: &str, call: &str) -> PyObject { … }

#[test]
fn nodes_match() {
    let mili = mili_rs::Database::open("tests/fixtures/basic1").unwrap();
    let rust_nodes = mili.nodes(MeshId(0));
    Python::with_gil(|py| {
        let py_nodes = oracle(py, "basic1", "db.nodes()");
        assert_arrays_eq(rust_nodes, py_nodes);
    });
}
```

This catches every semantic regression mili-python's own tests
catch, plus anywhere we silently misinterpret a byte. The price is
that CI needs Python + a built mili-python; we install it from the
`reference/mili-python/` submodule with `pip install -e .` in the
CI setup step.

Run it against every fixture under `reference/mili-python/tests/
data/serial/` — fifteen of them, including the critical
`dir_version_2/` fixture for v2 directory support. `basic1/` is
the smallest (9MB) and is the default for fast unit tests.

### Layer 2: C oracle for byte-level write parity (Phase 3 onward)

The C library is buildable in CI without HPC dependencies — CMake
+ BLT, no HDF5, no MPI. Fortran is optional and we turn it off:

```yaml
# .github/workflows/ci.yml fragment
- name: Build C oracle
  run: |
    cmake -S reference/mili -B build/c-oracle \
      -DENABLE_MILI=ON -DENABLE_TAURUS=OFF -DENABLE_EPRINTF=OFF \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_Fortran_COMPILER=NOTFOUND
    cmake --build build/c-oracle --parallel
```

We also write a tiny C harness `tools/c-oracle/write_canonical.c`
that emits a known database (a handful of meshes, svars, srecs,
and ~10 states with deterministic data). The Rust write tests then:

1. Call the same sequence through `mili-rs` to produce a Rust
   database.
2. Byte-compare the produced files against the C-emitted reference.

Acceptable diffs are limited to documented non-determinism — at
present, none. If we find any (e.g. timestamp fields), the C
harness gets a flag to zero them and we record the choice in
`shared/format.md`.

### Layer 3: fuzzing

`cargo-fuzz` targets on the parsers that touch untrusted byte
input — `header.rs`, `directory.rs`, the entry data type widening
in v1/v2 paths. Goal: an hour of fuzzing produces no panics, no
hangs, no OOMs. Any failures become regression tests in
`tests/regressions/`.

We do not fuzz the query path; its inputs come from already-parsed
metadata, so a query fuzzer would mostly be testing the metadata
fuzzer.

## CI shape

A single `.github/workflows/ci.yml` with these jobs:

1. **`lint`** — `cargo fmt --check`, `cargo clippy -- -D warnings`.
2. **`test-rust`** — `cargo test --workspace` on the smallest
   fixture set. Fast feedback (< 1 min target).
3. **`test-parity`** — installs `reference/mili-python` and runs
   the parity integration tests. Slower (~5 min). Required for
   merge.
4. **`test-write-parity`** (Phase 3 onward) — builds C oracle,
   runs round-trip byte-diff tests.
5. **`bench`** — manual trigger only; runs criterion on a fixed
   set of fixtures and posts the report as a workflow artifact.
6. **`fuzz`** (nightly cron) — runs each fuzz target for 30
   minutes.

The Rust toolchain is pinned via `rust-toolchain.toml` to a
specific stable so format/clippy lint sets don't drift mid-PR.

## Test fixtures

`reference/mili-python/tests/data/serial/` is our corpus. Phase 1
parity tests run against all 15 subdirectories; specific fixtures
target specific concerns:

| Fixture            | Purpose                                                  |
|--------------------|----------------------------------------------------------|
| `basic1`           | Smallest end-to-end DB; default for fast unit tests      |
| `dir_version_2`    | v2 directory format — must read without conversion       |
| `d3samp4`          | Larger element variety (hex/tet/quad/beam mix)           |
| `solids014`        | Hex-heavy mesh; element-count scaling                    |
| `tet`              | Tet-only mesh                                            |
| `beam`             | Beam superclass                                          |
| `vrt_BS`           | Vector and beam-stress svars                             |
| `vecarray`         | Vec-array aggregate type                                 |
| `labeling`         | Non-trivial label arrays                                 |
| `mstate`           | Multi-state-file partitioning                            |
| `rigid_body`       | Rigid body superclass corner case                        |
| `sstate`           | Single-state fixture (small)                             |
| `dbl_nodtang`      | Double-precision svars                                   |
| `fdamp1`           | Frequency-domain results                                 |

We avoid copying these into the `mili-rs` crate; tests resolve them
via the submodule path. CI's checkout step fetches submodules.

## Benchmarks

Criterion suite (`crates/mili-rs/benches/read.rs`) measures:

- `open` — wall time from `Database::open(path)` to the
  state_map being populated. Target: < 50 ms for any single-state-
  file fixture on a warm cache.
- `nodes` — `db.nodes()` materialization (always zero-copy native,
  so this is essentially mmap fault cost).
- `query_single` — single svar, all states, all entities. The hot
  path. Compared head-to-head against an equivalent mili-python
  call captured in the same harness via `pyo3`.
- `query_many` — same query with N svars; tests assembly overhead.

Pass criterion for Phase 1: `query_single` and `query_many` are
both ≥ 2× faster than mili-python on the corpus, with parity
results verified.

## Mandatory edge-case tests

Captured from `reference/mili-python/tests/test_bugfixes.py` and
the entry-payload survey. Every one of these has to pass before
Phase 1 is done. Each becomes a named integration test in
`crates/mili-rs/tests/edge_cases.rs`.

1. **Non-sequential mesh-object blocks.** A class's id range is
   split across multiple non-contiguous `CLASS_IDENTS` entries.
   The reader must coalesce them into the `id_blocks` vector and
   honor the layout when serving queries.
   (`test_bugfixes.py:25-38`.)
2. **Double-precision nodal positions.** A database where the
   `nodpos` svar is `M_FLOAT8` instead of `M_FLOAT4`. Confirms the
   precision-limit byte is honored and that
   `nodes()` returns `f64` rather than silently truncating.
   (`test_bugfixes.py:62-72`.)
3. **Vec-array with mixed component widths.** A vec-array whose
   components are a stress tensor (vector of 6 scalars) **plus** a
   scalar plastic-strain `eps`. Querying `"eps"` must extract
   only the scalar; querying `"sy"` must extract from the embedded
   stress sub-vector. The byte-offset math has to walk the
   component list, not multiply by a uniform width.
   (`test_bugfixes.py:119-172`.)
4. **Inconsistent integration-point counts across subrecords.**
   Different materials carrying the same svar with different IP
   counts. Querying without an `ips` filter must error; specifying
   `ips=4` filters to that single IP across all subrecords.
   (`test_bugfixes.py:99-117`.)
5. **Array-svar subscript notation.** Queries like `"hx[3]"` or
   `"hx[1-8]"` (1-based!) must resolve into the right slice of
   the underlying array svar. Out-of-range or 0-based indices
   raise a typed error. (`test_bugfixes.py:251-296`.)
6. **`dir_version_2` fixture.** Read v2 directory entries
   (4-byte ints) without converting on disk; widen to i64 in
   memory only.
7. **State end marker.** Write a small database via the C oracle
   (Phase 3) and confirm `mili-rs` round-trips the `~` byte in
   `R.A.tfile` byte-for-byte.

## Open questions to revisit during implementation

- **`block_obj_fmt` connectivity.** Does it ever appear in modern
  databases or is `list_obj_fmt` universal? If always list, we
  hold off on the block code path until the first failing fixture.
- **mmap on Lustre / NFS.** Defer the pread fallback until we have
  a failing benchmark; don't speculate-engineer it now.
- **String-pool UTF-8 strictness.** Current plan: validate once at
  parse time, fail on bad UTF-8. If a real-world fixture trips
  this, downgrade to `String::from_utf8_lossy` behind a
  `lossy_strings` feature flag and log a warning.
- **Public surface of `MiliBuffer`.** Lean toward keeping it
  pub(crate) for now; expose `Array3` / `ArrayView` instead. We
  can promote `MiliBuffer` if `mili-py` or `mili-viz` show a
  concrete need.

## Resolved questions

- **`PREC_LIMIT_DOUBLE` semantics** (resolved before step 1). The
  C lib's SINGLE and DOUBLE arms of `set_default_io_routines`
  (`dep.c:100-244`) populate `fam->external_size[]` identically;
  `M_FLOAT` is 4 bytes in both modes. Verified empirically on
  `dbl_nodtang` (header byte 7 = `0x02`, `db.nodes().dtype` is
  `float32`, only the explicit `M_FLOAT8` `nodtang` svar is
  `float64`). The Rust port resolves `M_FLOAT` to 4 bytes for both
  SINGLE and DOUBLE and rejects `NULL`/`QUAD`/`NONE` with a typed
  error. See `../shared/format.md` § Numeric types.
- **Format-v1 directory support** (resolved before step 2).
  **Defer with typed error.** The C writer can still emit v1 in
  principle (`direc.c:218-262`), but no v1 fixture exists in the
  corpus and the modern writer defaults to v3. Synthesizing one
  via the C oracle would require either coaxing the writer into a
  non-default mode (patches we don't take) or hunting for legacy
  databases. The marginal cost of adding v1 later is small: v1
  differs from v2 only in (a) the trailing header field
  `QTY_STATES` being absent — one less `i32` to read at trailer —
  and (b) it shares v2's 4-byte-int entry width and the same
  widen-to-LONGLONG path (`direc.c:519-537`). Step 2 lands v3
  first, v2 second, and emits `MiliError::UnsupportedDir(1)` on
  v1 input. If a v1 sample surfaces we extend in place.
- **Label/material trailing convention** (resolved before step 6).
  There is no trailing portion to split. `mc_def_conn_labels`
  (`mesh_u.c:1556-1678`) writes labels and local elem-ids as two
  separate TI arrays of length `qty` each. The `qty * 2`
  allocations at `mesh_u.c:1196` and `mesh_u.c:1473` are unused
  beyond the first half. Material numbers live entirely in
  separate `MAT_NAME_<n>` TI params. mili-python's reader
  (`miliinternal.py:96-106`) ignores `Element Labels-ElemIds*`
  entries and concatenates all `Element Labels*` entries per
  class identified by `Sname-(\w+)`. The Rust `labels(class)`
  accessor mirrors this. Split convention documented in
  `../shared/format.md` § TI_PARAM-as-storage pattern.

## What this plan deliberately does not cover

- The write path (Phase 3) — its own plan doc once the read path
  is stable.
- `MiliBuffer`-shaped public API trade-offs — those get pinned
  down once `mili-py` Phase 2 starts and surfaces real
  ergonomics constraints.
- ABI compatibility with `libmili` (`extern "C"` shims) — not on
  the roadmap.
