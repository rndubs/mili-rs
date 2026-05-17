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
inits the `reference/mili-python`, `reference/mili` **and**
`reference/griz` submodules (explicitly by path — never `--recursive`
or a bare `--init`; see below) and `pip install -e`s the Python oracle
(deps come transitively from its `pyproject.toml` — don't hand-maintain
a dep list). Idempotent; safe to re-run.

`.claude/hooks/session-start.sh` runs the script automatically in
Claude Code on the web (gated on `$CLAUDE_CODE_REMOTE`). Local sessions
must run it themselves before trusting a green run — a bare
`cargo test` without it silently skips the parity/xmilics coverage.

`reference/griz` holds the C reference source we cite by path (and the
source for the post-training grammar/intent corpus —
`planning/mili-viz/posttraining-dataset.md`). It is now checked out by
`scripts/setup-parity.sh`.

**Never `git submodule update --init --recursive` (or a bare
`--init`).** Only the three submodules above are wanted, by path. The
nested `reference/mili/test/mdgtest` is on an LLNL-internal SSH host
unreachable from CI / web runners, and recursing also dirties
`reference/mili` by bumping its vendored `cmake/blt` pin. Use
`scripts/setup-parity.sh` (or its explicit per-path
`git submodule update --init --depth 1 reference/<name>` commands).

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
