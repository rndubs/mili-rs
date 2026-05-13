# Shared design

Decisions that span more than one crate. If a fact about the mili format
or the buffer contract needs to be true for both `mili-rs` and `mili-py`
(or for the viz server), it belongs here, not duplicated.

## Documents

- `format.md` — reverse-engineered on-disk layout of a mili database.
  The source of truth for byte-level compatibility tests.
- `buffer.md` — the `MiliBuffer<T>` type that every layer holds and
  passes around. Defines the zero-copy contract that bridges mmap →
  Rust → numpy → wgpu.

## Cross-cutting decisions (summary)

### Workspace layout

A single cargo workspace at the repo root:

```
Cargo.toml          # [workspace]
crates/
├── mili-rs/        # core library
├── mili-py/        # PyO3 bindings (cdylib)
├── mili-viz-proto/ # shared command/RPC types
├── mili-viz-server/
└── mili-viz-client/
```

The `mili-viz-proto` split exists so the server and client can share
command definitions without the client pulling in `mili-rs` directly.

### Error model

`mili-rs` exposes a single `MiliError` enum via `thiserror`. Variants
distinguish I/O errors, format errors (bad magic, bad directory entry,
truncated state file), and semantic errors (unknown svar, state out of
range). The Python bindings translate `MiliError` into a small
hierarchy of exceptions that mirrors what mili-python currently raises,
to keep the existing test suite passing.

### Endianness

The mili header byte 6 declares file endianness. Readers decide at open
time whether the host matches; mismatched files require a per-buffer
byteswap. The byteswap is lazy and recorded on the `MiliBuffer` — see
`buffer.md`. We do not rewrite files on read.

### Test corpus

Phase 0 work item: gather a small set of reference databases that
exercise the format corners we care about — both endiannesses, both
subrecord orderings (RESULT_ORDERED, OBJECT_ORDERED), mixed element
classes, time-independent params, multiple state files. The
mili-python tests under `reference/mili-python/tests/` already point at
several such databases and become our initial corpus.

### Naming

- Rust crates: `mili-rs`, `mili-py`, `mili-viz-{proto,server,client}`.
- Python package name stays `mili` for drop-in compatibility (the
  PyO3 `cdylib` is named `mili._native`, re-exported by a thin Python
  shim).
- The viz binaries are `mili-viz-server` and `mili-viz` (the client).
