# Phase 6 M1 — landed (pygriz scaffold + connect/handshake)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- New top-level `python/` tree (the non-crate parallel of `crates/`)
  with `python/pygriz/` — distribution `pygriz`, import namespace
  `griz`, pure-Python (no pyo3/maturin), `src/`-layout, `setuptools`
  backend; not a Cargo workspace member.
- `scripts/gen-pygriz-stubs.sh` regenerates `griz._proto` from the
  one canonical `crates/mili-viz-proto/proto/mili_viz.proto` via
  `grpc_tools.protoc` and rewrites the top-level `import mili_viz_pb2`
  to package-relative `from griz._proto import …`. Stubs are
  gitignored build output (root `.gitignore` Python block cites
  Decisions 36 & 53), never hand-edited.
- `griz.connect(host, port, token=...)` completes the `Hello`
  handshake; mismatched `protocol_version` → warning (not exception),
  matching the Visit "API matches the engine" guarantee.
- Layer-0 escape hatch: `session.command(...)` and
  `session.run_script(path)` both send `Command{ raw }`;
  `run_script` sends the whole file byte-verbatim as one `raw`
  (the server's `parse_raw` does the splitting/comment-skipping —
  single-parser invariant).

## Gating test

`python/pygriz/tests/test_m1_connect.py` (run by `test-pygriz` job)
— `test_import_and_proto_pinned`, `test_connect_handshake_and_
layer0`, `test_handshake_mismatch_warns_not_raises`,
`test_run_script_is_one_verbatim_raw`; spawns the real
`mili-viz-server` binary on `127.0.0.1:0`, skip-on-absent when
`cargo`/the binary or `grpcio-tools` is unavailable.

## Decisions

- Decisions 35–37 (Phase 6 design slot) and 53–55 (M1 impl);
  index lives in [`status.md`](status.md).
