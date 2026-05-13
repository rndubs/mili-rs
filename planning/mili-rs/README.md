# `mili-rs` — core Rust library

A pure-Rust replacement for `libmili`. Byte-for-byte compatible with
existing mili databases on disk. No C dependency.

## Objectives

1. Read any database that `libmili` writes, on either endianness.
2. Write databases that `libmili` reads.
3. Parallelize the read path safely with `rayon` over the format's
   natural independence boundaries (state files, directory entries,
   subrecords within a state).
4. Expose results as `MiliBuffer<T>` (see `../shared/buffer.md`) so
   the bindings and viz server get zero-copy where the format allows.
5. Match or beat the mili-python read-path throughput on the test
   corpus before we declare Phase 1 done.

## Non-goals (Phase 1)

- The write path. Deferred to Phase 3.
- Derived results (principal stress, centroid, etc.). Those live in
  `mili-py` today and stay there until we have a reason to push them
  down.
- A C-ABI re-export for legacy callers. Not on the roadmap.

## Public surface (sketch)

```rust
pub struct Database { /* opaque */ }

impl Database {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, MiliError>;
    pub fn close(self);

    // Metadata
    pub fn meshes(&self) -> &[Mesh];
    pub fn classes(&self, mesh: MeshId) -> &[ObjectClass];
    pub fn svars(&self) -> &[Svar];
    pub fn subrecords(&self, srec: SrecId) -> &[Subrecord];
    pub fn states(&self) -> &[StateMeta]; // time, file, offset
    pub fn nodes(&self, mesh: MeshId) -> MiliBuffer<f32>;
    pub fn connectivity(&self, class: ClassId) -> MiliBuffer<i32>;
    pub fn labels(&self, class: ClassId) -> Option<MiliBuffer<i32>>;

    // Results — the hot path
    pub fn query(&self, q: Query) -> Result<QueryResult, MiliError>;
}
```

The `Query` type mirrors mili-python's `query()` arguments:
svar names, entity class, optional label / state / integration-point
filters. `QueryResult` is a `(states, entities, components)` shaped
`ndarray::Array3<T>` plus a layout block, identical in spirit to
mili-python's `QueryDict`.

## Module layout

```
crates/mili-rs/src/
├── lib.rs
├── error.rs
├── header.rs         # magic + endianness + version
├── directory.rs      # directory entry parsing
├── family.rs         # Database open/close, file handles
├── mesh.rs           # meshes, classes, nodes, connectivity, labels
├── svar.rs           # state variable definitions
├── srec.rs           # state records & subrecords
├── state.rs          # state file mapping & state metadata
├── query.rs          # the hot read path
├── buffer.rs         # MiliBuffer<T>
└── endian.rs         # byteswap helpers
```

## Phase 1 milestones

1. **M1 — header + directory.** Open `R.A`, parse the header, parse
   every directory entry. No data interpretation yet. Unit-tested on
   the smallest reference database.
2. **M2 — mesh metadata.** Classes, svars, subrecords, srecs, state
   index. No state reads. Pass parity tests against mili-python's
   `class_names()`, `labels()`, `state_maps()`, `times()`.
3. **M3 — node coordinates and connectivity.** First `MiliBuffer`
   returns. Parity vs. `mili.nodes()` and `mili.connectivity()` on
   the corpus.
4. **M4 — single-state, single-svar `query()`.** Serial. Covers
   both `RESULT_ORDERED` and `OBJECT_ORDERED`. Parity vs.
   mili-python on the simple cases in
   `reference/mili-python/tests/test_milidatabase.py`.
5. **M5 — full `query()` with label and state filters.** Still
   serial. Pass the rest of `test_milidatabase.py`'s read tests as
   a Rust integration test (we can shell out to Python to compute
   the oracle, or pre-bake oracle arrays as `.npy` fixtures).
6. **M6 — parallelize.** `rayon` over states and over subrecords
   within a state. Fix the O(N²) concat that mili-python suffers
   from by pre-allocating the result `Array3` and filling slices.
7. **M7 — benchmark.** Criterion benches over the corpus. Target:
   ≥ 2× mili-python read throughput, with zero-copy for the
   single-svar contiguous case.

## Open questions

- **mmap vs. pread.** Default to `memmap2` for state files; allow a
  fallback to buffered reads for filesystems where mmap is awkward
  (Lustre, certain NFS configs). Decision deferred until we
  benchmark on a real HPC mount.
- **String tables.** `M_STRING` blobs are NUL-terminated. We will
  expose them as `&str` after a UTF-8 check at parse time; invalid
  UTF-8 becomes a `MiliError::BadString` rather than lossy
  conversion.
- **TI parameters.** Time-independent params live in a parallel
  directory section. Probably worth a separate `ti.rs` module
  rather than overloading `directory.rs`.

## Validation strategy

- Unit tests per module on synthetic byte buffers.
- Integration tests open reference databases and assert against
  oracle arrays.
- A `cargo bench` suite running on the same corpus.
- A `cargo fuzz` target on the directory parser, since malformed
  directories are the most likely crash surface for a bad file.

## Things explicitly deferred

- Write path (Phase 3).
- TI map (`R.A.tio`) writing.
- Lock file management beyond honoring an existing lock.
- VisIt JSON sidecar generation.
