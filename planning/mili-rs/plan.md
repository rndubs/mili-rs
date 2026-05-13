# `mili-rs` implementation plan

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

- `Mesh` (one per `MESH_PARAM` entry chain).
- `ObjectClass` (one per `CLASS_DEF` entry): name, superclass code,
  element count, optional label array.
- `nodes(MeshId) -> MiliBuffer<f32>`: locate the `NODES` entry,
  return a buffer over the matching byte range.
- `connectivity(ClassId) -> MiliBuffer<i32>`: similar for
  `ELEM_CONNS`. Two on-disk formats (`M_LIST_OBJ_FMT`,
  `M_BLOCK_OBJ_FMT`); list form is straight zero-copy, block form
  needs a small materialization pass.

### `svar.rs`

Parses `STATE_VAR_DICT` entries into typed `Svar` records: name,
`num_type`, `agg_type`, rank, dims, optional components. Components
of vector svars are themselves svar IDs into the same table.

### `srec.rs`

State record format parser. One `Srec` per srec id; each holds a
`Vec<Subrecord>` with: class id, organization (`RESULT_ORDERED` /
`OBJECT_ORDERED`), svars referenced, `lump_offsets`, `lump_sizes`,
`lump_atoms` (matching the C `Sub_srec` shape).

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

TI param reader. Same directory parser as `directory.rs` but pointed
at the separate `R.ATI0…` files. Holds its own `Directory` and name
pool; reuses the body parsers for params and arrays.

## Incremental build order

The order below is what we land on the branch in sequence. Each step
is a PR-sized chunk that compiles, ships tests, and doesn't regress
prior work.

| Step | Lands                                                       | Validates against           |
|-----:|-------------------------------------------------------------|-----------------------------|
| 0    | Workspace, CI skeleton, `MiliError`, fixture symlinks       | `cargo test` green          |
| 1    | `header.rs` + golden bytes from `basic1.pltA`               | hand-checked offsets        |
| 2    | `directory.rs` for v3, then v2, then v1                     | parity on dir-entry count, names, types; `dir_version_2/` fixture |
| 3    | `family.rs` open path, `state_map` resolution               | parity on `times()`, state count |
| 4    | `mesh.rs`: nodes, connectivity, labels                      | parity on `nodes()`, `connectivity()`, `labels()` |
| 5    | `svar.rs`, `srec.rs`                                        | parity on `class_names()`, `svars()` |
| 6    | `buffer.rs` and `endian.rs`                                 | unit tests on synthetic mmaps |
| 7    | `query.rs` single-svar single-state, `RESULT_ORDERED`       | parity on simple `query()` cases |
| 8    | `query.rs` full filter set, `OBJECT_ORDERED`                | full mili-python read test suite |
| 9    | `ti.rs`                                                     | parity on TI param reads    |
| 10   | rayon over states; criterion benches                        | ≥ 2× mili-python throughput |
| 11   | cargo-fuzz targets on `directory.rs` and `header.rs`        | runs clean for an hour      |

Step 0 lands a working CI before any logic. Steps 1–6 land
read-side metadata; nothing returns data arrays yet. Step 7 is the
first end-to-end "user can read a result" milestone. Step 8 is the
Phase 1 exit criterion (full read parity). Steps 9–11 harden.

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

## Open questions to revisit during implementation

- **Format-v1 entry data type.** `direc.c:492-538` does a custom
  widening I haven't decoded byte-by-byte. Worth a one-day spike to
  produce a v1 fixture (if `dir_version_2` is the only old fixture,
  we may not have a v1 sample) and confirm the widening works.
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

## What this plan deliberately does not cover

- The write path (Phase 3) — its own plan doc once the read path
  is stable.
- `MiliBuffer`-shaped public API trade-offs — those get pinned
  down once `mili-py` Phase 2 starts and surfaces real
  ergonomics constraints.
- ABI compatibility with `libmili` (`extern "C"` shims) — not on
  the roadmap.
