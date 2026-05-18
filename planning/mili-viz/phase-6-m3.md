# `mili-viz` Phase 6 M3 — `pygriz` Layer-1 object API + the Layer-0 ≡ Layer-1 test (buildable scope)

> Scope doc for **Phase 6 Milestone 3**. M1
> ([`phase-6-m1.md`](phase-6-m1.md)) shipped the `pygriz` scaffold,
> the gitignored stub generator, `griz.connect(...)` + the `Hello`
> handshake, and the Layer-0 escape hatch (`session.command(...)` /
> `run_script(...)` → `Command.raw`). M2
> ([`phase-6-m2.md`](phase-6-m2.md)) shipped the connection model
> (`attach()`/`launch()`/`list_sessions()` + the server-side session
> file). M3 builds the **Layer-1 object API** from
> [`scripting.md`](scripting.md) § "API sketch" — the object API people
> should actually write — and carries the **Layer-0 ≡ Layer-1
> integration test** that pins the migration aid so it cannot drift.
>
> Read [`status.md`](status.md) first (the live tracker), then
> [`scripting.md`](scripting.md) (the two-layer model + the
> "Layer 0 ≡ raw command stream" requirement + the API sketch) and
> [`phase-6-m1.md`](phase-6-m1.md) (the single-parser invariant this
> generalizes). The decision log is global and monotonic across
> `mili-viz`; **M2 ended at Decision 58, M3 continues at 59**.

## Goal

Ship the Layer-1 object API on the landed `Session`:
`s.open/state/next/prev/first/last/select/show/isosurface/contour/
cutplane/colormap` + `s.selection`/`s.materials`/`s.legend` helpers +
`s.view.*` (server-authoritative), with the typed handles the
[`scripting.md`](scripting.md) sketch names (`Result`, `Isosurface`,
plus minimal `Database`/`Contour`). The defining invariant: **every
Layer-1 call lowers to the exact `Command` the raw Layer-0 stream
produces** — there is no second griz parser (nor emitter) in Python;
Layer-1 builds the typed `Command` oneof variants and the server's
existing dispatcher does all the work. **No change to the frozen
`mili_viz.proto` and no Rust/server change at all** — M3 is a pure
Python client addition (`crates/` is byte-for-byte untouched, so
`cargo test --workspace --exclude mili-py` is unaffected by
construction, exactly as in M2).

The `query`/`to_dataframe`/Arrow-Flight payoff is Phase 6 M5; the
`render`/`save_animation`/`snapshot` surface is M6; the continuous
`@s.on("state_changed")` live stream is M4. M3 is the scene/view
object API only.

## Decisions

### Decision 59 — Layer-1 is a pure typed-`Command` builder; every M3 call lowers to a typed oneof variant, none to `raw`

[`scripting.md`](scripting.md): "The Python layer lowers to the exact
proto the `egui` client emits … Layer-1 API calls lower to the typed
`Command` variants; `session.command(...)` / `run_script(...)` use the
`Command.raw` escape hatch, and the two MUST stay equivalent." The
frozen `Command` oneof (`crates/mili-viz-proto/proto/mili_viz.proto`)
has a typed arm for every griz verb the server's one parser
(`crates/mili-viz-server/src/raw.rs` `parse_line`/`to_raw`) handles —
the mapping is 1:1, so every sketch call has a typed home.

Resolution: each Layer-1 call sets protobuf fields directly and emits
the typed `Command` variant — `s.open(r)` → `Command{load{root}}`,
`s.state = n` → `Command{set_state{state}}`, `s.next()` →
`Command{step{dir=NEXT}}`, `s.show(r,c,**opts)` →
`Command{show{result,component,opts}}`, `s.view.set(...)` →
`Command{view{set{...}}}`, `s.view.save(n)` →
`Command{named_view{op=SAVE,name}}`, etc. **No Layer-1 call ever uses
the `raw` arm** — `raw` stays exclusively the Layer-0 escape hatch
(M1 Decisions 37 & 54). No griz string is ever *formatted* in Python:
the M1 single-*parser* invariant (the server's `parse_raw` is the one
parser) generalizes here to "there is also no second *emitter*" —
Python neither parses nor renders griz lines, it only fills typed
fields. This keeps the migration aid impossible to drift by
construction: there is one griz grammar, in the server, once.

### Decision 60 — the Layer-0 ≡ Layer-1 equivalence is asserted two ways: an always-on lowering pin + a skip-on-absent identical-session-effect leg

There is no Python griz parser to assert `parse(emit(c)) == c`
against (Decision 59 — by design). The server already proves
`parse_line ∘ to_raw == id` *and* dispatch equivalence in Rust
(`crates/mili-viz-server/tests/acceptance.rs` `layer0_equals_raw`,
`phase-4-m1.md`). The Python M3 obligation is therefore: (a) pin that
Layer-1 lowers to the *correct* typed `Command`, and (b) prove that
typed command and the equivalent raw line have *identical effect* on
the real server. Resolution — two halves, the CLAUDE.md / M1 / M2
skip-on-absent convention:

- **Always-on lowering pin (no server).** A fake stub captures every
  `Execute(Command)`. Each Layer-1 call asserts the exact oneof
  variant + fields (and that the `raw` arm is *never* used) — the
  lowering spec, pinned in Python so it cannot silently change. The
  server-authoritative reads (`Result.range`, `s.state`,
  `legend.limits`) are exercised against a fabricated `Subscribe`
  snapshot served by the same fake stub — still no server.

- **Skip-on-absent identical-session-effect (real server).** For each
  representative Layer-1 call, the equivalent Layer-0 griz line is
  written **by hand in the test** (never in the library — the library
  has no emitter/parser). Both are applied to two freshly
  `griz.launch()`ed real `mili-viz-server` processes; the authoritative
  session state read from the opening `DELTA_SNAPSHOT` of a short-lived
  `Subscribe` must be **byte-identical** between the two. The server's
  single dispatcher (`parse_raw`→typed | typed) is the only state
  owner, so equality proves the migration aid cannot drift end-to-end.
  Skipped (never failed) when `cargo`/the binary is unavailable —
  exactly the CLAUDE.md corpus skip M1/M2 use; the real
  `serial/basic1/basic1.pltA` fixture is used when present so a
  non-empty (`db`/`num_states`/`state`/`result`/`camera`) snapshot is
  compared, not a vacuous all-zero one.

### Decision 61 — server-authoritative reads use a one-shot `Subscribe` snapshot, never client prediction

[`scripting.md`](scripting.md) Decision 1: camera/view is
server-authoritative; a client is a peer that mutates and observes,
never a second state owner. The sketch nonetheless reads back state —
`r.range  # (min, max)`, `s.state = 10` (a property implies a getter).
M3 must satisfy these without becoming a second state model and
without the continuous live stream (that is Phase 6 M4's
`@s.on(...)`).

Resolution: `s.view.*` and every scene mutator **only emit** the typed
command; they never predict or reconcile locally (prediction/
reconciliation against the broadcast `DELTA_CAMERA` is Phase 5 M4,
client-side). The read-back accessors — `Result.range`, the `s.state`
getter, `s.legend.limits` — open a **short-lived `Subscribe`, take the
opening `DELTA_SNAPSHOT`'s authoritative fields, and close**. This is
the server's current truth read on demand: no cached client model, no
prediction, no second state owner — the minimal honest way to satisfy
the sketch's read-backs under the server-authoritative rule. The
typed handles (`Result`, `Isosurface`, `Database`, `Contour`) are
correspondingly thin: they re-emit typed `Command`s through their
owning `Session` (e.g. `Isosurface.remove()` → typed `iso off`) and
read state only via that one-shot snapshot — they hold no scene state.

## M3 acceptance gate

A gating test (`python/pygriz/tests/test_m3_layer1.py`, run by the
`test-pygriz` job), two halves mirroring the CLAUDE.md / M1 / M2
skip-on-absent convention:

- [x] **Always-on pure logic** (no server, no `cargo`): a fake stub
      captures the emitted `Command`; every scene + view Layer-1 call
      lowers to the exact typed oneof variant + fields and **never**
      the `raw` arm (Decision 59); `Result.range` / `s.state` /
      `s.legend.limits` read a fabricated `Subscribe` snapshot
      (Decision 61); the `show()`→`Result` / `isosurface()`→
      `Isosurface` (`.remove()`) / `open()`→`Database` /
      `contour()`→`Contour` handles are the documented thin shapes.
- [x] **Skip-on-absent** (spawns the real `mili-viz-server` TCP
      binary; skipped — never failed — when `cargo`/the binary is
      absent): for each of `state` / `step` / `show` / `select` /
      `view`, the typed Layer-1 call and its hand-written equivalent
      Layer-0 `s.command("<griz line>")`, applied to two freshly
      `launch()`ed servers, converge to byte-identical authoritative
      `DELTA_SNAPSHOT` state (Decision 60), against the real
      `basic1.pltA` fixture when present.
- [x] The frozen Phase 4 server acceptance suite + every Phase 5
      client gating test + the frozen Phase 6 M1 & M2 gates are
      **unchanged and green** — `crates/` is byte-for-byte untouched
      (no proto/`lib.rs`/server edit at all; `cargo test --workspace
      --exclude mili-py` green by construction), and
      `test_m1_connect.py` / `test_m2_attach.py` still pass.

## Out of scope for M3 (later Phase 6 milestones)

`Subscribe`/`@s.on(...)` continuous live sync + camera prediction/
reconciliation (M4); `db.query(...)` / `current_result.to_dataframe()`
/ Arrow Flight — the real data-back-into-Python payoff (M5);
`s.render()` / `s.save_animation()` / `s.snapshot()` via `CaptureFrame`
(M6); server-side token enforcement (a later hardening milestone, see
`phase-6-m2.md` Decision 56). None require a proto change — the M1
contract already froze all of it.
