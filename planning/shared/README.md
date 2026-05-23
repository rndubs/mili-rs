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

The fixtures under `reference/mili-python/tests/data/`,
`reference/mili/test/` (including `xmilics/`) and the v3
`reference/mili-python/tests/data/v3/` set serve as the corpus.
`scripts/setup-parity.sh` is the canonical setup; bit-exact parity
vs the `mili` Python oracle is gated by `crates/mili-rs/tests/
parity_*.rs` (the `parity` feature) and `crates/mili-py/tests/
test_upstream_readpath.py` (the redirect harness).

### Naming

- Rust crates: `mili-rs`, `mili-py`, `mili-viz-{proto,server,client}`.
- Python distribution / import: **`milox`** (`mili` + "ox" — the
  upstream `mili` name is taken on PyPI). The PyO3 `cdylib` is
  `milox._native`, re-exported by a thin Python shim.
- Scripting client: PyPI **`pygriz`**, import as **`griz`**.
- The viz binaries are `mili-viz-server` and `mili-viz` (the client).
