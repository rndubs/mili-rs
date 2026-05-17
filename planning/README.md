# mili-rs planning

This directory holds the design notes for a Rust rewrite of LLNL MDG's mili
ecosystem: the core C library (`reference/mili/`), its Python bindings
(`reference/mili-python/`), and the griz visualization client
(`reference/griz/`). The goal of the planning docs is to keep design state
out of chat and let each layer evolve in its own subdirectory while a small
shared layer captures cross-cutting decisions.

## What we are building

Three crates, developed sequentially, layered over a shared buffer
abstraction:

1. **`mili-rs`** — pure-Rust core library, byte-for-byte compatible with
   existing mili databases. Replaces `libmili`. See `mili-rs/`.
2. **`mili-py`** — PyO3 + numpy bindings that present the same Python API
   surface as the current `mili-python` package, but with zero-copy reads
   and parallel state assembly. See `mili-py/`.
3. **`mili-viz`** — a client/server replacement for griz. Server links
   `mili-rs` directly and does I/O, mesh prep, and parallel result
   computation; client renders with `wgpu` + `egui`. See `mili-viz/`.

Shared decisions (the buffer type, format compatibility test corpus,
naming, error model, workspace layout) live in `shared/`.

## Constraints driving the design

- **On-disk compatibility is non-negotiable.** Existing mili databases
  must round-trip. Format details and the reverse-engineered byte layout
  belong in `shared/format.md`.
- **Zero-copy where the format permits it.** The mili on-disk layout is
  already plain `f32`/`f64`/`i32` arrays at known offsets (see
  `shared/format.md`). The right "common type" is a refcounted typed byte
  buffer, not Apache Arrow. Arrow is reserved for the network transport
  in `mili-viz`.
- **Parallelism is opportunistic, not pervasive.** The format gives us
  independent byte ranges (separate state files, separate directory
  entries, separate subrecords). `rayon` over those ranges is safe;
  shared parser state is not. Writes stay serial.
- **HPC-friendly client.** The viz client must run on Linux, macOS, and
  Windows, including via remote rendering from an HPC server. `wgpu`'s
  Vulkan/Metal/DX12/GL backends cover this; `egui` matches griz's
  existing minimalist immediate-mode-ish UI.

## Phasing

The phases are sequential at the crate level but each phase has its own
internal milestones (documented in its subdirectory):

1. **Phase 1 — `mili-rs` read path. ✅ COMPLETE.** Open, query
   metadata, read state results. Parallel over directory entries.
   Bit-exact vs the mili-python oracle across the corpus. (`mili-rs/
   status.md`.)
2. **Phase 2 — `mili-py` (`milox`). ✅ COMPLETE.** PyO3 bindings
   presenting the `mili` API surface, backed by `mili-rs`. The
   upstream read-path test suite runs against `milox` with an import
   redirect: **938 pass / 0 xfail**, strict 0-xfail harness, 16/16
   upstream test-file coverage redirected-or-excluded. (`mili-py/
   m4.md` decision 25.)
3. **Phase 3 — `mili-rs` write path. ✅ COMPLETE.** `append_state`,
   `copy_non_state_data`, `query(write_data=)`, `AppendStatesTool` —
   bit-exact vs the upstream `AFileWriter` oracle; the last
   un-exercised writer edge (duplicate snames within a directory type)
   closed and gated. (`mili-py/phase-3.md`, `m4.md` decisions 22–26.)
4. **Phase 4 — `mili-viz` server. ⏳ NOT STARTED — needs more
   planning iterations before implementation.** Port griz's command
   interpreter as the RPC surface. In-process Rust client first, then
   split over Arrow Flight. Several design questions are still open;
   see [`mili-viz/status.md`](mili-viz/status.md).
5. **Phase 5 — `mili-viz` client. ⏳ NOT STARTED — gated on Phase 4
   M1.** `wgpu` + `egui` viewer; remote mode over Flight when the
   server runs on an HPC login node. See
   [`mili-viz/status.md`](mili-viz/status.md).

**Phases 1–3 are done: the port is functionally complete and
hard-gated against drift on both the Rust and Python sides.** Phases
1–2 unblocked the existing Python user base; Phase 3 unblocked
retiring `libmili`. **Phases 4–5 are the remaining work** and are a
new subsystem (a command-language server + renderer), not more
oracle-validated porting — they need their own design iterations
before coding starts. The single-source-of-truth for that is the
`mili-viz` status tracker.

## Layout

```
planning/
├── README.md            # this file
├── shared/              # cross-layer decisions
│   ├── README.md
│   ├── format.md        # on-disk mili format reference
│   └── buffer.md        # MiliBuffer / zero-copy contract
├── mili-rs/
│   ├── README.md
│   ├── plan.md           # detailed module-by-module build plan
│   └── status.md         # live status tracker (steps, edge cases, coverage)
├── mili-py/
│   ├── README.md
│   ├── m1.md … m4.md      # milestone scope + decisions (read path)
│   └── phase-i.md, phase-3.md  # parallel-handler + write-path slices
└── mili-viz/
    ├── README.md           # architecture (server/client split)
    ├── status.md           # ⭐ live tracker — START HERE for Phase 4/5
    ├── scripting.md        # scripting-client design (resolved)
    ├── client.md           # client wireframe + AI-first design (resolved)
    └── agent-local-llm*.md # local-LLM agent investigation
```

Each subdirectory's `README.md` is the entry point; further detail docs
get added alongside as questions get pinned down.

## Reference material

The three upstream projects live as git submodules under `reference/`:
`reference/mili/` (C library), `reference/mili-python/` (Python
bindings), `reference/griz/` (viewer). They are read-only — we are not
patching upstream. Specific file:line citations from the initial survey
are captured in the relevant component docs.
