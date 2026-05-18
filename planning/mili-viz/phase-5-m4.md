# `mili-viz` Phase 5 M4 — local view manipulation (buildable scope)

> Scope doc for Phase 5 Milestone 4, the wireframes'
> §"Implementation order" item 3. M1 stood up the `wgpu` renderer +
> orbit `Camera`; M2 drew the decoded server hull; M3 painted the
> `egui` shell and *emitted* a view command on `View reset`/`Fit`
> while keeping the camera **server-authoritative** (`scripting.md`
> Decision 1 — the server owns camera state, every client mirrors the
> broadcast `DELTA_CAMERA`). M3.5 added the bottom tabs. M4 adds the
> missing interactive half: **mouse orbit / drag / zoom with
> client-side prediction reconciled against the server-authoritative
> broadcast `DELTA_CAMERA`**, and finally honours `Colormap` /
> `LegendLimits` (deferred from `phase-5-m3.md` Decision 47). No proto
> change — the Phase 4 M1 contract is frozen and the server already
> implements every `View` op and broadcasts `DELTA_CAMERA`
> (`mili-viz-server/src/lib.rs:525` `apply_view`).
>
> Read [`status.md`](status.md) first, then
> [`scripting.md`](scripting.md) Decision 1 (camera authority),
> [`phase-5-m1.md`](phase-5-m1.md) Decisions 38–40 (the renderer
> skeleton + the `Camera` field-aligned to the proto `CameraState`),
> [`phase-5-m3.md`](phase-5-m3.md) Decision 47 (the deferred
> `Colormap`/`LegendLimits`), and
> [`phase-4-m1.md`](phase-4-m1.md) Decision 4 (the portable griz CLI
> subset, coded in the pre-M4 hardening below). Decision entries
> continue the **global, monotonic** log (Phase 4 ended at 34; Phase 6
> M1 35–37; Phase 5 M1 38–40, M2 41–43, M3 44–47, M3.5 48–52; Phase 6
> M2 56–58, M3 59–61). The last decision is **61**, so this doc starts
> at **62**: 62–63 are the two pre-M4 bug fixes, 64–66 the M4 scope.

## Pre-M4 hardening (two real bugs, landed before M4)

Two latent client bugs, neither tracked anywhere, were fixed first
because both block the manual-test loop M4 needs (a HiDPI window that
does not abort, and `-i <file>` that actually loads). They are not
their own milestone but they touch Phase 5 M1's frozen renderer
choice (Decision 39's `downlevel_defaults()`) and the never-coded
`phase-4-m1.md` Decision 4, so they are logged here with rationale.

### Decision 62 — keep `downlevel_defaults()` as the CI floor but raise `max_texture_dimension_2d` to the adapter's real maximum, and clamp the surface config + offscreen depth target to the negotiated maximum

`app.rs` and `renderer.rs` requested `wgpu::Limits::downlevel_defaults()`
(Decision 39's deliberate CI floor — a software/`llvmpipe` adapter on
a bare runner must satisfy the requested limits). `downlevel_defaults()`
caps `max_texture_dimension_2d` at **2048**. On a Retina/HiDPI display
the window's *physical* pixel size exceeds 2048, so `Surface::configure`
fails texture-size validation; the failure fires inside winit's macOS
`frame_did_change`, a `panic_cannot_unwind` context, so the process
**aborts** rather than surfacing a catchable error. The offscreen
render-to-texture depth target (sized to the surface) has the same
exposure.

**Decision: keep `downlevel_defaults()` as the floor but, after
selecting the adapter, raise *only* `limits.max_texture_dimension_2d`
to `adapter.limits().max_texture_dimension_2d` before
`request_device` — every real adapter reports ≥ 8192, and a
limited adapter still negotiates exactly what it can do (the CI floor
is unchanged for every other limit). Defensively `clamp(1, max_dim)`
the surface `SurfaceConfiguration` width/height on init **and** on
`WindowEvent::Resized`, and the renderer's offscreen depth `Extent3d`
to `device.limits().max_texture_dimension_2d`.** This is done in both
`app.rs` (the windowed device) and `renderer.rs` (`headless_device`,
so the gating tests' offscreen path mirrors the window). The clamp is
belt-and-suspenders: raising the limit alone fixes every real GPU, the
clamp guarantees no abort even on a hypothetical adapter whose true
maximum is still below the window size.

**Trade-off recorded.** A clamped surface on a (non-existent in
practice) sub-window-size adapter would render slightly low-res rather
than crash — strictly better than `panic_cannot_unwind`. Requesting
`adapter.limits()` wholesale was rejected: it would silently lift the
Decision-39 CI floor for *every* limit and let a regression that needs
a non-downlevel feature pass locally while CI's software adapter
rejects it — exactly the skip-on-absent foot-gun CLAUDE.md warns about.

### Decision 63 — the binary parses exactly the portable griz subset (`phase-4-m1.md` Decision 4); an unknown flag is a clear error, never a silent filename

`main.rs` passed `std::env::args().nth(1)` verbatim as the load root.
`phase-4-m1.md` Decision 4 specified the portable subset
(`-i <base>`, `-b`/`-batch <file>`, `-V`, `-w <w> <h>`) but it was
never coded, so the griz-muscle-memory `mili-viz-client -i file.pltA`
made `argv[1] == "-i"`; the server gracefully stubbed the unopenable
root (`db="-i"`, `num_states=1`, no geometry) and the viewport came up
blank with no error.

**Decision: a new pure, GPU-free `cli::parse_args` (the always-on test
core, M1-Decision-40 pattern) implements exactly the Decision-4
subset, hand-rolled (no `clap` — the grammar is four flags; a CLI
parser crate is unjustified weight on a binary that takes a path).
`-i <base>` and a bare positional both set the load root (two roots →
a clear error, not last-wins); `-V`/`-version`/`--version` prints
`mili-viz-client <CARGO_PKG_VERSION>` and exits 0; `-b`/`-batch <file>`
and `-w <w> <h>` are parsed into `CliArgs` and currently logged as
explicit no-ops (the startup-script runner is Phase 6-gated; honouring
`-w` waits on a windowed size hook — neither is silently swallowed);
any other `-flag` is a one-line error to stderr + exit 2 listing the
accepted subset.** Bare positional still works so every existing
manual-test invocation in CLAUDE.md keeps working unchanged.

**Trade-off recorded.** Accepting-and-ignoring unknown griz flags
(`-s`, `-u`, …) would smooth wrapper-script migration; rejected for
the same reason as Decision 4 — those toggle Motif/GLX behaviour that
does not exist here, and a clear error beats a silent mystery
(Decision 4's own trade-off, inherited).

## Goal (M4 proper)

Wireframes §"Viewport interaction": the central viewport responds to
the mouse —

- **Left-drag → orbit** (azimuth/elevation).
- **Right-drag (or middle-drag) → pan** (focus translate).
- **Scroll wheel → zoom** (distance).

…with **client-side prediction**: the drag updates the local
`Camera` *immediately* (no round-trip latency — critical on an HPC
WAN, M5's territory) and *simultaneously* emits the matching frozen
`View` command. The server applies it authoritatively and broadcasts
`DELTA_CAMERA`; the client **reconciles** its predicted camera against
that broadcast (`scripting.md` Decision 1 — the server owns camera
state; a second client, a script, or `view reset` must still move
*this* client's view). M4 also finally honours `Colormap` and
`LegendLimits` (deferred from `phase-5-m3.md` Decision 47).

Out of scope (unchanged from the wireframes' split): the AI panel
(M6); **remote** mode + HPC-latency buffer tuning (M5 — M4 stays on
the in-process transport); picking/selection-by-click (the status-bar
`pick` readout stays `—`, a later milestone).

## Decisions (continuing the global log)

### Decision 64 — predict-and-reconcile: the local `Camera` is updated optimistically on input and *also* emits the frozen `View` op; every `DELTA_CAMERA` broadcast is authoritative and overwrites the predicted camera

The mouse handler mutates `App.camera` immediately (prediction —
responsive even at WAN latency) and pushes the matching `View` op
(`Rotate`/`Translate`/`Zoom`) through the existing `Session::execute`
seam, exactly as M3 already does for `ViewReset`/`Fit`. `ingest_deltas`
grows a `state_delta::Payload::Camera` / `Snapshot.camera` arm that
maps the proto `CameraState` field-for-field onto `App.camera`
(Decision 40 deliberately shaped `Camera` 1:1 with `CameraState` so
this is a field copy, not a conversion: `azimuth/elevation/distance`
direct, `fx/fy/fz → focus`; `fov_y/z_near/z_far` are client-only
projection params the proto does not carry and are preserved).

**Reconciliation policy: last-broadcast-wins, unconditional.** The
predicted camera is overwritten by every `DELTA_CAMERA` (including
ones this client caused — `origin_client_id` is *not* used to suppress
self-echo). Rationale: the server's `apply_view` is a pure function of
the ops it received in order, so a self-caused broadcast equals the
prediction up to f64 rounding — overwriting is idempotent and removes
an entire class of drift/ordering bugs, and it is the *only* behaviour
that keeps a script's or a second client's `view` command correctly
authoritative over a mid-drag local prediction. No client-side
sequence buffer / op-replay (rejected as premature: in-process and M5
TCP both deliver `DELTA_CAMERA` within one frame of the `Execute`
reply; the visible artefact of last-wins is at most one frame of
already-applied prediction being re-set to an equal value).

### Decision 65 — the client is radians end-to-end and treats `CameraState`/`View.Rotate` as unit-agnostic scalars; the proto's "degrees" comment is non-normative (the server is the unit authority and it is a plain add)

The proto comments `Rotate` as "degrees", but the server's
`apply_view` does `s.camera.azimuth += r.x` — a unit-agnostic add, and
`CameraState` is whatever the accumulated ops put there. The client's
`Camera` is radians (`phase-5-m1.md` Decision 40). **Decision: the
client uses radians for `azimuth/elevation` on *both* the wire and
locally — it sends `Rotate{ x: Δaz_rad, y: Δel_rad }` and reads
`CameraState.azimuth` straight into `Camera.azimuth` (radians).**
Because the server only ever *adds* deltas and *echoes* the
accumulator, and `pygriz` (Phase 6 M3) lowers `view` to the same typed
`View` ops without re-interpreting units, the whole system is
self-consistent in radians; the "degrees" comment is documentation
debt on a frozen proto we must not touch (recorded here, not fixed).
Mouse → camera gains: a full viewport-width left-drag = π rad azimuth
(180° feel), elevation clamped by the existing `Camera::eye` pole
guard; scroll = geometric distance scale (`Zoom{ factor }`,
`distance /= factor`); right-drag pan = `Translate` in focus-space
scaled by `distance` so the grab point tracks the cursor.

### Decision 66 — `Colormap` and `LegendLimits` are honoured client-side: the colormap name selects the viz-local ramp and `LegendLimits` overrides the autoscale range used by `upload_mesh` and the legend overlay

`phase-5-m3.md` Decision 47 mapped the `MVG2` scalar through a single
hard-coded cool→warm ramp autoscaled by `ResultState.{min,max}` and
explicitly deferred `Colormap`/`LegendLimits` here. **Decision: a
`UiAction::SetColormap(name)` / `SetLegendLimits(min,max)` (lowering to
the frozen `Command::Colormap`/`LegendLimits` oneofs the server
already handles) and a viz-local named-ramp table (`cool`, `warm`,
`grayscale`, `hot` — a small fixed set; an unknown name falls back to
cool→warm with a logged note, never an error). The effective range fed
to `renderer::upload_mesh` and the legend overlay is
`LegendLimits` when either bound is set, else the `ResultState`
autoscale — a pure `ShellState` method (always-on tested), so the
render seam is unchanged.** The colormap ramp stays a client concern
(no scalar re-fetch — `upload_mesh` already re-colours from the cached
`Mesh.scalars`); a `LegendLimits` change re-runs `upload_mesh` with
the new range, no new geometry round-trip.

## Open question (needs a decision before coding M4)

**File→Open / interactive load (Part-1 item 3 — usability gap, not a
bug).** `shell.rs:347–359`'s menu buttons are empty stubs
(`ui.menu_button(m, |_| {})`) and there is no file dialog, so a run is
only loadable via argv or the Layer-0 `load` command in the bottom
tab. A real `Control → Open…` needs a native file picker; the
idiomatic `egui` choice is the `rfd` crate (≈ one small dep, pulls a
platform dialog backend). This is **not** in the wireframes' M4 scope
("local view manipulation"); recorded here as an explicit gap with a
recommendation to **defer** to its own tiny milestone (or fold into
M5, where remote `connect`/`attach` already reworks the load path)
rather than add a GUI-dialog dependency mid-M4. **Pending the
maintainer's call** (do not add `rfd` without it).

## Acceptance gate

- `cli::parse_args` unit tests (always-on): `-i`/bare positional set
  the root, `-V` → version, `-b`/`-w` parse not error, unknown flag
  errors, missing value errors, two-roots errors. ✅ landed.
- A pure `ShellState`/`Camera` reconcile test (always-on): a
  `CameraState` broadcast overwrites a predicted `Camera`
  field-for-field; a `LegendLimits` override changes the effective
  range while the autoscale path is unchanged when unset.
- Skip-on-absent composite render unchanged (the `egui`/mesh seam,
  Decision 45, byte-stable — M4 adds input handling and a delta arm,
  not a render-path change).
- `cargo test --workspace --exclude mili-py` green; the frozen Phase 4
  server suite + Phase 5 M1/M2/M3/M3.5 + Phase 6 M1/M2/M3 gates green;
  `cargo fmt --check` + `cargo clippy` clean; `mili_viz.proto`
  byte-untouched.
