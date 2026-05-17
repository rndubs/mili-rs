# Phase 3 — the milox/mili-rs write path

> **Status: 3.1 + 3.2 + 3.3 LANDED — Phase 3 complete.** Self-contained
> entry-point doc for the write slice (mirrors
> [`phase-i.md`](phase-i.md)). The summary + **decisions 22/23/24** live
> in [`m4.md`](m4.md) § "Phase 3"; this file carries the reproducible
> starting state so a fresh session can pick it up cold. Read
> [`m4.md`](m4.md) (decisions 18/19/20/21 + 22/23/24) and
> [`../mili-rs/status.md`](../mili-rs/status.md) (the `milox` parity +
> redirect tracker row + § "Surprises") first. The write path is the
> last unported redirect surface — with 3.3 landed there is no
> remaining milox redirect work. **Closeout (decision 25, `m4.md`):**
> the redirect-coverage audit then wired the last two read-path modules
> (`test_utils`/`test_bugfixes`) and consciously excluded the 3 non-API
> ones (`test_imports`/`test_afileIO`/`test_plotting`), added a
> `test_redirect_coverage_is_exhaustive` + strict 0-xfail
> `pytest_sessionfinish` hard gate in CI, and fixed the last latent
> read gap it surfaced (`Subrecord.state_byte_offset`). milox 900 → 938
> pass / 0 xfail; upstream-file coverage 16/16.

## Why this phase exists

Phase I (I.1–I.4) closed the parallel read path; the milox suite was
**827 pass / 6 xfail**, the 6 = `test_append_state` /
`test_copy_non_state_data` × {Serial,Server,Loop}WrapperReductions in
`test_reductions` (`_REDUCTIONS_WRITE_METHODS`, reason "Phase 3 write
path"). The write direction (`append_state`, `copy_non_state_data`,
`query(write_data=)`, `AppendStatesTool`) is the last unported
surface. The Rust core was READ-ONLY (mmap over A/T/S, no writer).

## Decision 22 (recorded in [`m4.md`](m4.md) § "Phase 3")

The on-disk A/T/S writer lives in the **Rust core**
(`mili_rs::write`), gated by
`crates/mili-rs/tests/parity_write_append.rs` (diffs milox-written
bytes vs upstream-`mili`-`AFileWriter`-written bytes on the d3samp6
corpus). Decision 19 applied to the write direction: writing the
on-disk format **is** parity-sensitive byte layout, so it gets its
own `parity_*.rs` gate — not a Python re-implementation of layout.
The verbatim-Python alternative (port `afileIO`) was rejected: it
also needs a second parallel `AFileParser`, duplicating the core's
single-source-of-truth parsing (contradicts decision 19). See
[`m4.md`](m4.md) for the full trade-off + the empirical bound (the
payload-copy finding).

## The empirical finding that bounds the core writer

Upstream `AFileParser.parse` → `AFileWriter.write` is **not** a
byte-identity round-trip — it **renormalises**: directories reorder
into a fixed `dir_order`, the string pool is rebuilt in write order,
offsets/lengths/dir-decls are recomputed, `STATE_VAR_DICT` int/char
streams are rebuilt, smaps move, the footer is rewritten. So milox
must reproduce upstream `AFileWriter`'s **output**, not the original
`.A`. Measured on every d3samp6 fragment:

- **42/42** non-`STATE_VAR_DICT` directory payloads
  (MILI/APP/TI_PARAM, CLASS_DEF, CLASS_IDENTS, NODES, ELEM_CONNS) +
  **SREC_DATA** are emitted **byte-identical to the original `.A`
  mmap payload byte-ranges** (grouped by sname, original
  first-occurrence order within each type).
- The 16-byte header is also a verbatim original-range copy.
- Only **`STATE_VAR_DICT`** is rebuilt (upstream renormalises its
  int/char stream — verified divergent from the original payload),
  plus the string pool + dir-decls + smap block + footer.
- No d3samp6 fragment has duplicate snames within a directory type,
  so "one payload write per decl, all decls emitted" = the merged
  set (the upstream stale-offset-on-duplicate-sname quirk does not
  arise for this corpus; if a future corpus has it, reproduce
  `__write_directories`'s first-decl-only offset update). **Phase 3.2
  update:** the `serial/sstate` d3samp6 corpus *also* has **no**
  duplicate snames (empirically verified — the Phase-3.1 framing that
  it did was a misdiagnosis; the serializer was already bit-exact on
  it). The real append-to-existing-states blocker was the stale
  in-memory model, fixed in 3.2 (`status.md` § Surprises).

So `mili_rs::write` copies payload byte-ranges verbatim and rebuilds
only `STATE_VAR_DICT` (port of `AFileWriter.__write_svars` /
`__collect_svar_data`) + the string pool + dir-decls + smaps +
footer.

## What 3.1 implements (the upstream contract)

`reference/mili-python/src/mili/`:

- `miliinternal.py:1433` `append_state(new_state_time, zero_out=True,
  limit_states_per_file=None, limit_bytes_per_file=None) -> int`:
  validates time-monotonic + subrecords-exist, computes
  `state_size = sum(srec.byte_size)+8`, the new smap (file/offset by
  the limit rules), appends the smap + bumps APP_PARAM `state_count`
  if present, rewrites the `.A` via `AFileWriter`, writes the state
  file (`'wb'` if a new state file else `'rb+'`, seek to offset):
  `struct.pack('fi', time, 0)` + zeroed body (`zero_out`/0-state) or
  the copied previous-state body; then on a 0-state/`zero_out` db
  writes `nodpos` = `self.__nodes` and `sand` = `1.0` via
  `query(..., write_data=)`. Returns `len(smaps)`.
- `miliinternal.py:1542` `copy_non_state_data(new_base_name)`:
  `afile.copy_non_state_data()` (clears smaps) → `AFileWriter.write`;
  appends the source basename's trailing `(\d+)$` digits to
  `new_base_name` (per-proc disambiguation). Returns `None`.
- `afileIO.py:492` `AFileWriter` — the serializer ported (per
  decision 22, into `mili_rs::write`, bounded by the payload-copy
  finding).
- `milidatabase.py:846/870` thin wrappers → `__postprocess(...,
  reductions.zeroth_entry)`.
- `parallel.py` `LoopWrapper`/`ServerWrapper` forward
  `[proc.append_state(...) for proc in procs]` (decision-21 per-proc
  list of `open_single` `_MiliInternal`).

The 6 xfailed tests (`test_reductions.py:188/232`) only assert the
return value (`1` / `None`); the real validation is
`parity_write_append.rs`.

## Phased plan (each its own parity-validated PR)

### Phase 3.1 — `append_state` + `copy_non_state_data`  ✅ LANDED

`mili_rs::write` (A-file serializer + `copy_non_state_data` +
`append_state` incl. the state-file write + `nodpos`/`sand` patch);
`PyMiliDatabase.copy_non_state_data` / `append_state` FFI (Single
backend; the wrapper opens per-fragment `open_single`); milox
`_MiliInternal` auto-forwards via `__getattr__`,
`MiliDatabase._REDUCE_FUNCTIONS` already maps both to
`zeroth_entry`. New `crates/mili-rs/tests/parity_write_append.rs`
(copy_non_state_data bit-exact ×8 fragments; append_state `.A` +
state-file bit-exact vs the upstream `_MiliInternal` golden, with
skip-not-fail on absent submodule/oracle and full corpus cleanup).
`test_append_states.py` wired into the harness `_REDIRECTED` and
promoted where bit-exact; the 6 `_REDUCTIONS_WRITE_METHODS`
promoted. `test_modify_database` / `test_append_states_tool`
wired as honest strict-xfail (concrete reason → 3.2 / 3.3).
Write tests create `*.plt*` under cwd and `os.remove` them in
tearDown — mirrored; no generated artifacts committed.

### Phase 3.2 — `query(write_data=)` write-half  ✅ LANDED

`mili_rs::Database::scatter_query` (decision 23,
[`m4.md`](m4.md) § "Phase 3"): the read `ReadPlan` **inverted** to an
`rb+` per-state byte-slab scatter — the byte-for-byte inverse of
`run_query`'s gather and the generalisation of 3.1's single-svar
`scatter_state_field` to an arbitrary svar / class / labels / states
/ ips from a `QueryDict`. The `write_data`-label → result-order
realignment upstream spells `argsort`/`searchsorted`
(`miliinternal.py:1331-1334`) is the plan-result-label → `wd_labels`
position map; the state axis is positional (`sidx`). Thin FFI in
`database.rs::query` (primal svars, `Single` backend; the parallel
wrappers fan out per-fragment per decision 21). `append_state` made
in-memory-refreshing (`&mut self` + `Database::reload`; the live-
mmapped `.A` rewritten atomically write-then-rename; the `zero_out`
+ `n>0` nodpos patch now copies the **previous state**, mirroring
upstream `miliinternal.py:1518-1538`). `milox.utils` gained verbatim
`results_by_element` / `writeable_from_results_by_element`
(`milox.reader` re-exports them); `state_maps()` carries
`state_map_id`. Gated bit-exact by
`crates/mili-rs/tests/parity_write_query.rs`. **All 26
`test_modify_database` + all 18 `test_append_states` promoted** →
milox 837 → 877 pass / 63 → 23 xfail (the 23 = `test_append_states_tool`,
Phase 3.3).

### Phase 3.3 — `mili.append_states.AppendStatesTool`  ✅ LANDED

Verbatim Python port (decision 24 in [`m4.md`](m4.md)) of upstream
`src/mili/append_states.py` (366 lines) into `milox.append_states`,
only the `mili.*` imports repointed at `milox`. It is pure input-spec
validation (`VALID_OUTPUT_TYPES` / `VALID_OUTPUT_MODES`,
`states`/`state_times`/`time_inc`, `limit_*`, per-svar shape/jagged/
int-point checks) + orchestration over the Phase-3.1/3.2 bit-exact
`copy_non_state_data` / `append_state` / `query(write_data=)`
primitives — no new byte-layout kernel, so decision 18/19 applies (the
inverse of 22/23). Surfaced + fixed three latent milox read-path gaps
(nested `svar.svars[]` population so VECTOR/VEC_ARRAY `atom_qty` is
non-zero; element-set-name `ips=` interpreted as IP *labels*; a
state-less no-tfile `copy_non_state_data` output reopens as a valid
0-state db). Behaviourally gated by the redirected
`test_append_states_tool.py` (all **23** cases — 19 invalid-input +
serial/parallel `append`/`write` — promoted); byte-gated by the new
`crates/mili-rs/tests/parity_write_append_states_tool.rs` (upstream
`mili` tool vs `milox` tool, `.A` + state files diffed on
serial/sstate, `append` + `write` modes). milox 877 → 900 pass /
23 → 0 xfail.

## Reproducible environment / commands

```
scripts/setup-parity.sh                       # submodules + pip-installs the mili oracle (idempotent)
pip install pytest ./crates/mili-py           # maturin-builds + installs milox (rebuild after Rust changes)
python -m pytest -q crates/mili-py/tests      # milox parity + redirect suite (827 pass / 6 xfail at Phase-I close)
cargo test --workspace --exclude mili-py --features parity   # Rust incl. parity_write_append.rs
cargo fmt --check && cargo clippy --workspace --exclude mili-py --features parity
```

Single redirected module:
`python -m pytest -q crates/mili-py/tests/test_upstream_readpath.py -k "append_states"`.

## Key files (the starting-point map)

| Concern | Path |
|---|---|
| Read-only core / Database open + accessors | `crates/mili-rs/src/family.rs` |
| Directory + name pool model (writer input) | `crates/mili-rs/src/directory.rs` |
| Svar model (only rebuilt payload) | `crates/mili-rs/src/svar.rs` |
| Srec model (raw-copied payload) | `crates/mili-rs/src/srec.rs` |
| Header (verbatim-copied 16 B) | `crates/mili-rs/src/header.rs` |
| **The writer** | `crates/mili-rs/src/write.rs` |
| **Parity gates** | `crates/mili-rs/tests/parity_write_append.rs` (3.1), `…parity_write_query.rs` (3.2), `…parity_write_append_states_tool.rs` (3.3) |
| FFI write methods | `crates/mili-py/src/database.rs` |
| milox wiring (auto-forward + reduce) | `crates/mili-py/python/milox/{miliinternal,milidatabase,parallel,reader}.py` |
| Redirect harness + xfail buckets | `crates/mili-py/tests/test_upstream_readpath.py` |
| Upstream write contract | `reference/mili-python/src/mili/{miliinternal,afileIO,milidatabase}.py` |
| Upstream tests | `reference/mili-python/tests/test_{append_states,modify_database,append_states_tool,reductions}.py` |
| Byte layout | `planning/shared/format.md`, `planning/shared/entry-payloads.md` |
| Trackers | `planning/mili-rs/status.md` (milox row), `m4.md` § "Phase 3" |

## Update protocol

Each Phase 3.x lands as its own commit/PR with: the parity gate
green, the milox suite count bumped in
[`../mili-rs/status.md`](../mili-rs/status.md) + [`m4.md`](m4.md)
§ "Phase 3", any corpus-vs-doc surprise logged per
`status.md` § "Surprises", the harness xfail set narrowed (never a
silent pass; a still-different case stays honestly xfailed with a
concrete reason; never delete a case).
