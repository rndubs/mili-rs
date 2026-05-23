# Phase 6 M3 — landed (pygriz Layer-1 object API + Layer-0 ≡ Layer-1 test)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- Layer-1 object API on the landed `Session`: `s.open`, `s.state`,
  `s.next`/`prev`/`first`/`last`, `s.select`, `s.show`,
  `s.isosurface`, `s.contour`, `s.cutplane`, `s.colormap`, plus
  `s.selection`/`s.materials`/`s.legend` helpers and `s.view.*`
  (server-authoritative), with the typed handles the
  `scripting.md` sketch names — `Result`, `Isosurface`,
  `Database`, `Contour`.
- Pure typed-`Command` builder: each call sets protobuf fields
  directly and emits the typed oneof variant (e.g. `s.show(r,c)` →
  `Command{show{...}}`, `s.view.set(...)` → `Command{view{set}}`).
  **No Layer-1 call ever uses the `raw` arm** — `raw` stays
  exclusively Layer-0's escape hatch. No griz string is parsed or
  *formatted* in Python: M1's single-parser invariant generalizes
  to "no second emitter either".
- Read-back accessors (`Result.range`, `s.state` getter,
  `s.legend.limits`) open a short-lived `Subscribe`, take the
  opening `DELTA_SNAPSHOT`'s authoritative fields, and close. No
  cached client model, no prediction (camera reconcile is Phase 5
  M4 client-side). Typed handles are correspondingly thin and hold
  no scene state.
- `crates/` is byte-for-byte untouched (no proto/`lib.rs`/server
  edit at all); `cargo test --workspace --exclude mili-py` green
  by construction.

## Gating test

`python/pygriz/tests/test_m3_layer1.py` (run by `test-pygriz`) —
always-on lowering pin (a fake stub captures each `Execute(Command)`,
asserts the exact oneof variant/fields, and that `raw` is never
used) + skip-on-absent identical-session-effect leg (typed Layer-1
call and its hand-written equivalent `s.command("<griz line>")`
applied to two freshly `launch()`ed servers must converge to
byte-identical `DELTA_SNAPSHOT` state, against `basic1.pltA` when
present).

## Decisions

- Decisions 59–61; index lives in [`status.md`](status.md).
