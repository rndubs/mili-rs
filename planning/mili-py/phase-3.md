# `mili-py` Phase 3 — landed (the milox/mili-rs write path)

> **Status: ✅ COMPLETE.** Full decision history and implementation
> notes live in [`m4.md`](m4.md) (decisions 22–26). This file is
> retained for cross-references; the body has been collapsed.

## What landed

- **3.1 — `append_state` + `copy_non_state_data`.** The on-disk
  A/T/S writer landed in the Rust core (`mili_rs::write` — decision
  22): bounded by the empirical finding that upstream
  `AFileParser`→`AFileWriter` is **not** a byte-identity round-trip
  but renormalises everything except payload byte-ranges, so the
  writer raw-copies all non-`STATE_VAR_DICT` payloads and rebuilds
  only the svar dict + string pool + dir-decls + smaps + footer.
  FFI on `PyMiliDatabase` (Single backend; wrappers fan out
  per-fragment via `open_single`); milox `_MiliInternal`
  auto-forwards.
- **3.2 — `query(write_data=)` write-half.** `Database::scatter_query`
  (decision 23) — the read `ReadPlan` inverted to an `rb+` per-state
  byte-slab scatter, byte-for-byte inverse of `run_query`'s gather;
  `append_state` made in-memory-refreshing (`&mut self` +
  `Database::reload`; live-mmapped `.A` rewritten via
  write-then-rename; `zero_out` + `n>0` nodpos patch copies the
  previous state). `milox.utils` got verbatim
  `results_by_element` / `writeable_from_results_by_element`.
  Promoted all 26 `test_modify_database` + all 18
  `test_append_states` cases.
- **3.3 — `mili.append_states.AppendStatesTool`.** Verbatim Python
  port (decision 24) of the 366-line upstream module into
  `milox.append_states` — pure input-spec validation + orchestration
  over the 3.1/3.2 primitives, no new byte-layout kernel. Surfaced +
  fixed three latent milox read-path gaps (nested `svar.svars[]` so
  VECTOR/VEC_ARRAY `atom_qty` is non-zero; element-set `ips=` as IP
  *labels*; a state-less no-tfile `copy_non_state_data` output
  reopens as a valid 0-state db). All 23 `test_append_states_tool`
  cases promoted.
- **Closeout (decision 25).** Redirect-coverage audit: wired the
  last two read-path modules (`test_utils` / `test_bugfixes`),
  consciously excluded the 3 non-API ones
  (`test_imports` / `test_afileIO` / `test_plotting`), added a
  `test_redirect_coverage_is_exhaustive` + strict 0-xfail
  `pytest_sessionfinish` hard gate in CI, and fixed the last latent
  read gap (`Subrecord.state_byte_offset`).
- **Decision 26 — duplicate-sname handling.** Whole-corpus audit
  found 57 fixtures with duplicate snames within a directory type
  (ELEM_CONNS ×53, CLASS_DEF ×4; none in d3samp6). The upstream
  stale-offset-on-duplicate quirk is now reproduced bit-exact in
  `mili_rs::write` rather than left as prose (duplicates merged into
  one payload per sname; only the first decl's offset/length/strings
  are updated; later duplicates keep their stale offset/length;
  duplicate in any other directory type is a hard error).
- **Net.** milox **827 → 938 pass / 6 → 0 xfail**; upstream-file
  coverage 16/16.

## Gating tests

- `crates/mili-rs/tests/parity_write_append.rs` (3.1).
- `crates/mili-rs/tests/parity_write_query.rs` (3.2).
- `crates/mili-rs/tests/parity_write_append_states_tool.rs` (3.3).
- `crates/mili-rs/tests/parity_write_dup_sname.rs` (decision 26;
  5 fixtures byte-diffed vs upstream `AFileWriter` golden).
- `crates/mili-py/tests/test_upstream_readpath.py` (the redirect
  harness; `_REDUCTIONS_WRITE_METHODS` + `test_append_states` +
  `test_modify_database` + `test_append_states_tool` all promoted).

## Decisions

- Decisions 22–26 recorded in [`m4.md`](m4.md).
