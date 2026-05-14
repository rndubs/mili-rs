# `mili-py` — PyO3 + numpy bindings

> **Status: ready to start.** Phase 1 (`mili-rs`) is feature-complete
> for the read path and has bit-exact parity against `mili-python`
> across 12 corpus fixtures (`crates/mili-rs/tests/parity_*.rs`).
> See [`../mili-rs/status.md`](../mili-rs/status.md) for the Phase-1
> wrap-up and the "Known gaps Phase 2 inherits" section before
> starting M1.

A drop-in replacement for the existing pure-Python `mili` package,
backed by `mili-rs` under the hood. The Python-facing API is the same;
the implementation is Rust.

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
from mili import open_database
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
│   ├── lib.rs         # #[pymodule] mili._native
│   ├── database.rs    # PyMiliDatabase wrapping mili_rs::Database
│   ├── query.rs       # PyQuery + result conversion
│   ├── arrays.rs      # MiliBuffer → numpy bridge (capsule + Pod)
│   └── errors.rs      # MiliError → Python exception hierarchy
└── python/mili/
    ├── __init__.py    # re-exports + thin shims
    ├── derived.py     # derived results (ported from upstream)
    └── ...
```

The Rust `cdylib` is named `mili._native`. User-facing imports stay
`from mili import open_database`; the `__init__.py` re-exports the
Rust types and keeps the Python-only helpers (derived results,
post-processing utilities) where they already live.

## Zero-copy plan

For each result buffer:

1. `mili-rs` returns a `MiliBuffer<T>`.
2. `arrays.rs` decides: aligned + native-endian → wrap in numpy via
   `PyArray::borrow_from_array` with a capsule destructor that drops
   the `Arc<Storage>`. Else → `to_owned()` and transfer.
3. The `QueryResult` shape `(states, entities, components)` is built
   in Rust as a single `Array3<T>`; numpy sees it directly via
   `IntoPyArray`.

The `Arc<Storage>` keeps the mmap alive as long as numpy holds a
reference, which matches what scientific users actually do (open
database, pull many arrays, close at exit).

## Phase 2 milestones

1. **M1 — `open_database`, metadata accessors.** `class_names()`,
   `labels()`, `times()`, `state_maps()`. Stand up the PyO3 module
   and the test harness.
2. **M2 — `nodes()`, `connectivity()`.** First zero-copy arrays
   across the FFI boundary.
3. **M3 — basic `query()`.** Single svar, single state. Matches
   mili-python's output exactly on the test corpus.
4. **M4 — full `query()`.** All filter combinations. Run
   `reference/mili-python/tests/` unmodified against the new module.
5. **M5 — performance.** Microbench the assembly path; confirm we
   are not regressing simple cases and we are winning on the
   multi-subrecord case.
6. **M6 — packaging.** `maturin` build, wheels for the platforms
   the Python users actually use (linux x86_64 + aarch64, macOS arm64,
   probably skipping Windows for v1 unless asked).

## Open questions

- **Writeability.** Python users mutate query results in place
  occasionally. `np.frombuffer` arrays in current mili-python are
  read-only; we keep that behavior. If a user passes
  `copy=True`-equivalent, they get a writable copy. Need to confirm
  no test relies on writability of returned arrays.
- **Derived results.** Some derived results need access to several
  primal arrays; the current Python implementations live in
  `reference/mili-python/src/mili/derived.py`. They should still work
  unchanged on top of our primal arrays, but we need to verify the
  dtype / shape assumptions match.
- **`MiliDatabaseSet` / parallel-wrapper helpers.** The upstream
  package has a `Mili` factory that returns either a single database
  or a fanned-out wrapper across multiple plot files. We provide the
  same factory; the per-database object is Rust-backed.
