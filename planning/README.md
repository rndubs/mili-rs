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

1. **Phase 1 — `mili-rs` read path.** Open, query metadata, read state
   results. Parallel over directory entries. Validate against the
   mili-python test suite as an oracle.
2. **Phase 2 — `mili-py`.** PyO3 bindings exposing the existing
   `MiliDatabase` surface, backed by `mili-rs`. Migrate the
   mili-python test suite; it becomes our regression net.
3. **Phase 3 — `mili-rs` write path.** Parity with `mc_wrt_*`. Round-trip
   tests against reference databases.
4. **Phase 4 — `mili-viz` server.** Port griz's command interpreter as
   the RPC surface. In-process Rust client first, then split over Arrow
   Flight.
5. **Phase 5 — `mili-viz` client.** `wgpu` + `egui` viewer; remote mode
   over Flight when the server runs on an HPC login node.

Phases 1 and 2 unblock the existing Python user base. Phase 3 is
required before anyone retires `libmili`. Phases 4–5 can start in
parallel with Phase 3 once `mili-rs`'s read API stabilizes.

## Layout

```
planning/
├── README.md            # this file
├── shared/              # cross-layer decisions
│   ├── README.md
│   ├── format.md        # on-disk mili format reference
│   └── buffer.md        # MiliBuffer / zero-copy contract
├── mili-rs/
│   └── README.md
├── mili-py/
│   └── README.md
└── mili-viz/
    └── README.md
```

Each subdirectory's `README.md` is the entry point; further detail docs
get added alongside as questions get pinned down.

## Reference material

The three upstream projects live as git submodules under `reference/`:
`reference/mili/` (C library), `reference/mili-python/` (Python
bindings), `reference/griz/` (viewer). They are read-only — we are not
patching upstream. Specific file:line citations from the initial survey
are captured in the relevant component docs.
