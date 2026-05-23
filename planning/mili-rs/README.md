# `mili-rs` — core Rust library

A pure-Rust replacement for `libmili`. Byte-for-byte compatible with
existing mili databases on disk. No C dependency.

> **Status: ✅ COMPLETE (read + write).** Phases 1, 1.5 and 3 have
> all landed — the core reads any `libmili` database bit-exact and
> writes back through the renormalising `AFileWriter`-compatible
> serializer, both gated against the upstream oracle. No open work in
> the `mili-rs` core; next work is Phase 4/5
> ([`../mili-viz/status.md`](../mili-viz/status.md)).
>
> **Where to look:** [`plan.md`](plan.md) for the module-by-module
> build plan, [`status.md`](status.md) for the live tracker (now the
> historical Phase-1/3 record).

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

## Non-goals

- A C-ABI re-export for legacy callers. Not on the roadmap.

*(The "derived results live in `mili-py`" non-goal from the original
Phase-1 plan was revisited in Phase 2/3 — `mili_rs::derived` now
hosts the parity-sensitive value kernels, with the listing surface
in `milox`; see [`status.md`](status.md) § "Surprises".)*

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

## Milestones — ✅ all landed

The original Phase 1 milestones (M1 header/directory through M7
benchmark) all landed in Steps 0–16; Phase 1.5 (Steps 17–19) added
multi-A-file orchestration (`DatabaseSet`), xmilics-corpus parity,
and the numpy/rayon FFI integration plan; Phase 3 (the write
path) landed `mili_rs::write` (renormalising A-file serializer +
`copy_non_state_data` + `append_state` + `scatter_query` +
`AppendStatesTool`), bit-exact vs the upstream `AFileWriter`
oracle. Per-step status lives in [`plan.md`](plan.md) §
"Phasing" and [`status.md`](status.md).

## Validation strategy

- Unit tests per module on synthetic byte buffers.
- Integration tests open reference databases and assert against
  oracle arrays (`mili` Python package via pyo3, `--features parity`).
- A `cargo bench` suite running on the same corpus.
- `cargo fuzz` targets on the directory / header / param parsers.

## Still deferred

- Directory v1 (`MiliError::UnsupportedDir(1)`).
- `SURFACE_CONNS` payload decode.
- `block_obj_fmt` connectivity (`list_obj_fmt` is universal in our
  corpus).
- mmap-on-NFS / Lustre `pread` fallback (no benchmark motivates it).
- VisIt JSON sidecar generation.
