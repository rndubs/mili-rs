"""milox parallel-handler wrappers — per-proc method forwarding.

Upstream (`reference/mili-python/src/mili/parallel.py:19-356`) selects
``LoopWrapper`` (serial-over-fragments, ``suppress_parallel``) or
``ServerWrapper`` (subprocess-per-proc) as the per-database engine and
forwards **every public read method** to the per-proc list
``[proc.method(*a, **kw) for proc in procs]``. ``MiliDatabase`` then
applies ``reductions.combine`` when ``merge_results=True`` and returns
the raw per-proc list when ``False``.

In milox the MPI fan-out **and** the ``merge_results=True`` merge live
in the Rust ``DatabaseSet`` (decision 19); these wrappers hold the
single already-opened ``PyMiliDatabase`` (the ``Set`` backend) and
adapt the upstream forwarding contract onto the Phase-I.1
``*_per_fragment()`` FFI accessors — the *direct* per-proc primitive
chosen for exactly this purpose (decision 20, shape option (a):
``db.<m>_per_fragment(...)`` *is* upstream's
``[proc.<m>(...) for proc in procs]``).

Forwarding is ``merge_results``-gated:

* ``merge_results=True`` → forward to the **merged** single-``Set``
  accessor (the Rust ``DatabaseSet`` already performed upstream's
  per-fragment reduction; the Python ``__postprocess`` combine is
  identity — decision 19). Unchanged from the prior marker behavior,
  so every already-green ``merge_results=True`` parallel test (e.g.
  ``test_reductions``'s ``TestServerWrapperReductions`` /
  ``TestLoopWrapperReductions``, opened with ``merge_results=True``)
  stays exactly as before.
* ``merge_results=False`` → forward the methods the
  ``merge_results=False`` per-proc contract requires to their
  ``*_per_fragment()`` sibling (the upstream per-proc list shape).

**Phase I.2 scope boundary (per `planning/mili-py/phase-i.md` —
promotion is I.4):** I.2's green target is the redirected
``test_grizinterface`` (4 cases) + the ``grizinterface`` port.
``GrizInterface.__init__`` consumes the per-proc shape for exactly
``class_names`` (``processor_count = len(...)`` + flatten),
``state_maps``/``mesh_dimensions``/``srec_fmt_qty`` (``[0]``),
``parameters`` (per-proc iteration in ``merge_parameters`` /
``load_free_node_data``) and ``connectivity_ids`` /
``mesh_object_classes`` / ``subrecords`` (``db._mili.*`` direct
reads); every other ``GrizInterface`` field is merely stored, so the
merged shape satisfies it. Methods outside that set keep the merged
shape under ``merge_results=False`` too — so the standing
``_MDB_PARALLEL_CLASSES`` xfail bucket (``merge_results=False``) does
**not** incidentally flip (its ``state_maps`` / ``mesh_object_classes``
assertions still legitimately differ — raw per-proc dicts/tuples are
not the upstream ``StateMap`` / ``Dict[str,MeshObjectClass]`` shape).
The full per-proc surface + bucket promotion is Phase I.4; the
``merge_results=True`` re-reduce is Phase I.3.
"""

from __future__ import annotations

from typing import Any, Dict

from .datatypes import ReturnCode

# Wrapper method name -> the Phase-I.1 ``*_per_fragment()`` accessor
# that yields the upstream ``[proc.method() for proc in procs]`` list.
# Scoped to the ``GrizInterface.__init__`` per-proc contract (I.2);
# the remaining accessors land + their xfail buckets promote in I.4.
_PER_FRAGMENT: Dict[str, str] = {
    "class_names": "class_names_per_fragment",
    "state_maps": "state_maps_per_fragment",
    "mesh_dimensions": "mesh_dimensions_per_fragment",
    "srec_fmt_qty": "srec_fmt_qty_per_fragment",
    "parameters": "parameters_per_fragment",
    "connectivity_ids": "connectivity_ids_per_fragment",
    "mesh_object_classes": "mesh_object_classes_per_fragment",
    "subrecords": "subrecords_per_fragment",
}


class _EngineWrapper:
    """Hold the single ``PyMiliDatabase`` (``Set`` backend) and forward
    public reads either per-proc (``merge_results=False``, the
    ``*_per_fragment()`` accessors) or merged (``merge_results=True``).

    **Phase I.3 decision (recorded — the I.3 re-reduce relocation):**
    ``merge_results=True`` forwards every read to a ``_MiliInternal``
    adapter *over the ``Set``-backed ``PyMiliDatabase``*. The Rust
    ``DatabaseSet`` already performed upstream's per-fragment reduction
    bit-exactly (decision 19; ``parity_xmilics``/``database_set``
    fixtures gate it), and ``_MiliInternal`` supplies the exact upstream
    accessor signatures + return shapes (``labels(class_name)``,
    ``times()`` → ``ndarray``, the return-code plumbing, …). So the net
    ``merge_results=True`` result is the *same merged value* as before
    (Rust merge, untouched) now wearing the upstream
    ``MiliDatabase``-method shape — **no Python re-merge of core data**
    (the decision-point's "keep the Rust merge where bit-exact, don't
    double-work"; upstream's ``__postprocess``-applies-``reduce_function``
    maps in milox to *the Set backend already being reduced* and
    ``_MiliInternal`` reshaping it). ``reductions.combine`` /
    ``merge_result_dictionaries`` / ``reduce_*`` are the full verbatim
    port (``milox.reductions``) used by the redirected
    ``test_reductions`` collection + the ``ResultModifier`` path; they
    are not invoked to re-reduce the already-merged Set accessors.

    ``merge_results=False`` keeps the Phase-I.2 per-fragment routing
    (scoped to the ``GrizInterface.__init__`` contract; the full
    per-proc surface + xfail-bucket promotion is Phase I.4)."""

    def __init__(self, db: Any, merge_results: bool = True) -> None:
        self._db = db
        self._merge_results = merge_results
        self._merged: Any = None
        if merge_results:
            # Lazy import: reader.py imports both parallel and
            # miliinternal; importing at module scope would order-couple
            # them. _MiliInternal(db) wraps the already-opened
            # Set-backed PyMiliDatabase (its __init__ takes the engine
            # directly when handed a PyMiliDatabase).
            from .miliinternal import _MiliInternal

            self._merged = _MiliInternal(db)

    def __getattr__(self, name: str) -> Any:
        # Reached only when the attribute is not found normally.
        if self.__dict__["_merge_results"]:
            return getattr(self.__dict__["_merged"], name)
        db = self.__dict__["_db"]
        per_fragment = _PER_FRAGMENT.get(name)
        if per_fragment is not None:
            return getattr(db, per_fragment)
        return getattr(db, name)

    def returncode(self) -> Any:
        """``merge_results=True``: surface the inner ``_MiliInternal``'s
        return code so its ERROR/CRITICAL codes raise through
        ``MiliDatabase.__postprocess`` exactly as upstream. Otherwise
        the per-fragment path raises in the Rust core directly, so there
        is no per-proc code to surface — always OK."""
        if self._merge_results:
            return self._merged.returncode()
        return (ReturnCode.OK, "")

    def clear_return_code(self) -> None:
        if self._merge_results:
            self._merged.clear_return_code()

    def close(self) -> None:
        """No subprocesses to tear down (fan-out is in Rust)."""


class LoopWrapper(_EngineWrapper):
    """Serial-over-fragments handler identity (suppress_parallel)."""


class ServerWrapper(_EngineWrapper):
    """Parallel handler identity. ``close()`` is the documented
    teardown hook upstream callers invoke."""
