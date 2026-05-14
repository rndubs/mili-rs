# mili-rs working notes for Claude Code sessions

## Fixture tests skip-on-absent

The integration tests under `crates/mili-rs/tests/*_fixtures.rs` open
files inside `reference/mili-python/tests/data/...`. That tree is a git
submodule. **If the submodule is not checked out, those tests early-
return rather than failing** — so a local `cargo test` reads as "all
green" while CI (which fetches submodules) catches the regression.

Before trusting a green `cargo test` run, make sure the submodule is
populated:

```
git submodule update --init reference/mili-python
```

`.claude/hooks/session-start.sh` runs this automatically in Claude Code
on the web sessions (gated on `$CLAUDE_CODE_REMOTE`). Local sessions are
on their own.

The other two submodules (`reference/mili`, `reference/griz`) hold C
reference source we cite by path only — there is no need to check them
out for `cargo test` or `cargo clippy`.

## Where the implementation plan lives

- `planning/mili-rs/plan.md` — module-by-module plan and incremental
  build order.
- `planning/mili-rs/status.md` — live tracker; flip a step to ✅ when
  it lands and bump the test counts.
- `planning/shared/format.md` and `planning/shared/entry-payloads.md` —
  on-disk byte layout reference. When the corpus and the doc disagree
  (it has happened — `CLASS_DEF`'s superclass is in `MODIFIER2`, not
  `MODIFIER1`), the corpus wins and the doc gets a fix-up entry under
  `status.md` § "Resolved questions log".
