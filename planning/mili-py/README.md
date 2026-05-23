# `mili-py` — PyO3 + numpy bindings

> **Status: ✅ COMPLETE.** The `milox` PyO3/numpy bindings present the
> full `mili` API surface backed by `mili-rs`. The upstream read-path
> test suite runs against `milox` with only an import redirect:
> **938 pass / 0 xfail**, strict 0-xfail harness, 16/16 upstream
> test-file coverage redirected-or-excluded. Milestones M1–M4 + the
> Phase-I parallel-handler slice + the Phase-3 write path all landed;
> see [`m4.md`](m4.md) (decisions 1–26) and [`phase-3.md`](phase-3.md)
> for the record. No open work in the `mili-py` port — next work is
> Phase 4/5 ([`../mili-viz/status.md`](../mili-viz/status.md)).

An API-compatible reimplementation of the pure-Python `mili` package
(upstream `mili-python`), backed by `mili-rs` under the hood. The
Python-facing API is the same; the implementation is Rust. Published
on PyPI as **`milox`** (`mili` + ox — oxidized; the upstream `mili`
name is taken by an unrelated project) and imported as `milox`
(`import milox`). The method/return surface mirrors `mili-python` so
existing code ports with only the import line changed.

## Objectives

1. Zero-copy numpy arrays for the contiguous, native-endian common
   case.
2. Fix the assembly hot path. The current mili-python
   `query()` does `np.concatenate` across subrecords inside a tight
   loop (`reference/mili-python/src/mili/miliinternal.py:1414-1416`),
   which is O(N²) for large element sets. We pre-allocate in Rust
   and fill `&mut [T]` slices in parallel.
3. Keep the entire mili-python test suite passing without
   modification.

## Non-goals

- Reshaping the user-facing API. We are not redesigning `query()`;
  the cost of breaking downstream notebooks is not worth a marginally
  prettier shape.
- Reimplementing derived results in Rust on day one. Derived results
  stay in Python over the Rust primal results until we have a profile
  showing they need to move.

## Public surface

Identical to the current `MiliDatabase`:

```python
from milox import open_database
db = open_database("/path/to/run")
db.query(svar_names, entity_type, material=..., labels=..., states=..., ips=...)
db.nodes()
db.times()
db.labels()
db.connectivity()
db.state_maps()
db.materials()
db.material_numbers()
db.class_names()
db.element_sets()
db.integration_points()
db.supported_derived_variables()
db.geometry  # property
```

The top 15 methods from the existing API are pinned here; the full
audit is in `reference/mili-python/src/mili/miliinternal.py`. We do a
final diff pass against the upstream API at the start of Phase 2.

### Full `query()` signature

`reference/mili-python/src/mili/milidatabase.py:770-844`:

```python
def query(self,
    svar_names: Union[List[str], str, ...],
    entity_type: Union[str, EntityType],
    material:        Optional[Union[str,int]]   = None,
    labels:          Optional[Union[List[int],int]] = None,
    states:          Optional[Union[List[int],int]] = None,
    ips:             Optional[Union[List[int],int]] = None,
    write_data:      Optional[Dict[str, QueryDict]] = None,
    as_dataframe:    bool                        = False,
    modifier:        Optional[ResultModifier]    = None,
    project_to_nodes: bool                       = False,
    **kwargs) -> Union[Dict[str, pd.DataFrame], Dict[str, QueryDict]]
```

Hidden `**kwargs` validated at `miliinternal.py:1159`:
`output_object_labels`, `subrec`, `source` (`'primal'` /
`'derived'`), `reference_state`, `face`. Anything else raises.
`modifier` is a `ResultModifier` enum: `MIN`, `MAX`, `AVERAGE`,
`MEDIAN`, `STDDEV`, `CUMMIN`, `CUMMAX`. Negative state indices
(e.g. `-1` for last) are supported.

The Rust binding accepts the same signature. `write_data`,
`as_dataframe`, `modifier`, and `project_to_nodes` are
post-processing wrappers around the basic primal query; for v1
they can stay in Python on top of the Rust primal call. We push
them into Rust only if profiling shows it matters.

### Filename root parsing
(`reference/mili-python/src/mili/reader.py:19-71`,
`afileIO.py:34-57`)

`open_database()` accepts:

- `/path/to/run` — the basename, no suffix.
- `/path/to/run.plt` — also accepted; trailing `.plt` is stripped.
- `/path/to/run.plt00` — also accepted.
- For parallel files (`dblplt00A`, `dblplt01A`, …), pass `dblplt`;
  the loader uses regex `re.escape(base) + r"(\d*)A$"` to find
  matching A-files and sorts them numerically.

Not accepted: glob patterns, directory paths. Missing directory →
`MiliFileNotFoundError`.

The Rust binding has to replicate this normalization in Python (it
predates the FFI boundary). The Rust core's `Database::open` takes
a fully-resolved `&Path`; the Python shim does the regex / suffix
stripping.

### Parallel wrappers
(`reference/mili-python/src/mili/parallel.py:19-356`,
`milidatabase.py:65-88`)

Three operating modes:

| Mode                                 | Wrapper            |
|--------------------------------------|--------------------|
| single A-file                        | `_MiliInternal`    |
| multiple A-files, `suppress_parallel=True` | `LoopWrapper`      |
| multiple A-files (default)           | `ServerWrapper`    |

`LoopWrapper` and `ServerWrapper` proxy every public method of
`_MiliInternal` via runtime method-forwarding. `ServerWrapper`
spawns one worker per core and uses shared memory for the
large-array returns. With `merge_results=True` (default), results
are reduced via `reductions.py` helpers (`list_concatenate_unique`,
`dictionary_merge_no_concat`).

For Phase 2, the Rust-backed `_MiliInternal` slots into all three
wrappers unchanged — the wrappers introspect the underlying
object's methods, which still work when those methods are PyO3
bindings. We do not rewrite the wrappers in Rust.

### Exception hierarchy
(`afileIO.py:27-30`, `milidatabase.py:36-38`)

Three exceptions all derived from `Exception`:

| Python class             | Rust `MiliError` variant(s)                              |
|--------------------------|----------------------------------------------------------|
| `MiliFileNotFoundError`  | `Io(NotFound)` after path normalization fails            |
| `MiliAParseError`        | `BadMagic`, `UnsupportedHeader`, `UnsupportedDir`, `Truncated`, `DirEntryOutOfRange`, `BadName` |
| `MiliPythonError`        | `UnknownSvar`, `UnknownClass`, `StateOutOfRange`, `Misaligned`, and any query-level validation error |

Mapping happens in `crates/mili-py/src/errors.rs` via a `match`
that converts each `MiliError` variant to the right Python class.

## Crate shape

```
crates/mili-py/
├── Cargo.toml         # crate-type = ["cdylib"]
├── src/
│   ├── lib.rs         # #[pymodule] milox._native
│   ├── database.rs    # PyMiliDatabase wrapping mili_rs::Database
│   ├── query.rs       # PyQuery + result conversion
│   ├── arrays.rs      # MiliBuffer → numpy bridge (capsule + Pod)
│   └── errors.rs      # MiliError → Python exception hierarchy
└── python/milox/
    ├── __init__.py    # re-exports + thin shims
    ├── derived.py     # derived results (ported from upstream)
    └── ...
```

The workspace crate stays `mili-py` (`crates/mili-py/`); the built
PyPI distribution and the importable package are both **`milox`**
(`pip install milox`, `import milox`). The Rust `cdylib` is the
private extension module `milox._native`; `python/milox/__init__.py`
re-exports the Rust types and keeps the Python-only helpers (derived
results, post-processing utilities). `pyproject.toml` sets
`[project] name = "milox"` and `tool.maturin.module-name =
"milox._native"`.

## Zero-copy plan

> **Superseded by [`../mili-rs/plan.md`](../mili-rs/plan.md) § "FFI
> integration plan (Phase 1.5 — Step 19)".** That section is the
> authoritative contract (pinned after this README was drafted). The
> capsule/`borrow_from_array` design below is the *deferred M5 path*,
> not the default. Summary of the resolved decision:
>
> - **Default return:** `IntoPyArray::into_pyarray_bound(py)` on an
>   owned `Vec<T>` / `ndarray::Array<T,_>` — numpy adopts the heap
>   buffer, no byte copy. This is what every `query()` / `nodes()` /
>   `connectivity()` return uses unless profiling says otherwise.
> - **`ToPyArray`** only when ownership cannot transfer (borrowed
>   `&[T]`); avoid on the hot path.
> - **`Arc<Mmap>` + `PyCapsule` zero-decode view:** deferred to M5,
>   profiling-driven. Only wins when aligned + native-endian +
>   single contiguous slab (rare in practice).

Original `MiliBuffer<T>` + `PyCapsule` + `Arc<Storage>` sketch
deferred to the M5 path above; see
[`../mili-rs/plan.md`](../mili-rs/plan.md) § "FFI integration plan"
for the full design.

## Phase 2 milestones (all landed)

1. ✅ **M1 — `open_database` + metadata accessors** ([`m1.md`](m1.md)).
   PyO3 module + maturin scaffold, `PyMiliDatabase` over
   `Database`/`DatabaseSet`, read-only metadata, error hierarchy,
   dedicated `test-milox` CI job.
2. ✅ **M2 — `nodes()` + `connectivity()`** ([`m2.md`](m2.md)). First
   bulk arrays across the FFI boundary via the owned-`Vec` →
   `into_pyarray_bound` zero-copy path under `allow_threads`.
3. ✅ **M3 — basic `query()`** ([`m3.md`](m3.md)). The pinned upstream
   `QueryDict` dict shape, single- and multi-state / multi-svar,
   bit-exact over the parity corpus + xmilics families.
4. ✅ **M4 — full `query()` + upstream test redirect**
   ([`m4.md`](m4.md), decisions 16–19 and the full record). All filter
   combinations; the upstream `reference/mili-python/tests/` suite
   runs against `milox` with only an import redirect — closed at
   542 pass / 287 xfail before Phase I.
5. ✅ **Phase I — parallel per-proc-unmerged surface**
   ([`phase-i.md`](phase-i.md), decisions 20–21). Brought milox to
   827 pass / 6 xfail.
6. ✅ **Phase 3 — write path** ([`phase-3.md`](phase-3.md), decisions
   22–26). The on-disk A/T/S writer in `mili_rs::write`,
   `append_state` / `copy_non_state_data` / `query(write_data=)` /
   `AppendStatesTool`. Closed at 938 pass / 0 xfail, strict 0-xfail
   harness, 16/16 upstream test-file coverage.

Original M5 (performance microbench) and M6 (multi-platform wheel
packaging) plan items were not gated by the read+write parity work
and are not blocking; revisit if/when downstream consumption needs
them.

## Open work

None — Phase 4/5 is the next work (see
[`../mili-viz/status.md`](../mili-viz/status.md)).
