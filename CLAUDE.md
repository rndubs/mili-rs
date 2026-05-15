# mili-rs working notes for Claude Code sessions

## Parity / fixture tests skip-on-absent — run `scripts/setup-parity.sh`

Many integration tests read fixtures from submodules and **early-return
instead of failing when the data or the `mili` Python package is
absent** — so a local `cargo test` reads "all green" while CI catches
the regression. This affects:

- `crates/mili-rs/tests/*_fixtures.rs` and `database_set_fixtures.rs`
  → `reference/mili-python/tests/data/...`
- `crates/mili-rs/tests/parity_*.rs` (the `parity` feature)
  → the `mili` Python package as oracle, via pyo3
- `crates/mili-rs/tests/parity_xmilics.rs`,
  `tests/smoke_full_corpus.rs` → also `reference/mili/test/xmilics/...`

**There is one canonical, repeatable setup — use it everywhere:**

```
scripts/setup-parity.sh
cargo test --workspace --features parity
```

`scripts/setup-parity.sh` is the single source of truth shared by CI's
`test-parity` job, the web session-start hook, and local developers. It
inits the `reference/mili-python` **and** `reference/mili` submodules
and `pip install -e`s the Python oracle (deps come transitively from
its `pyproject.toml` — don't hand-maintain a dep list). Idempotent;
safe to re-run.

`.claude/hooks/session-start.sh` runs the script automatically in
Claude Code on the web (gated on `$CLAUDE_CODE_REMOTE`). Local sessions
must run it themselves before trusting a green run — a bare
`cargo test` without it silently skips the parity/xmilics coverage.

`reference/griz` holds C reference source we cite by path only — not
needed for any test.

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
