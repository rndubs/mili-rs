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
    ``*_per_fragment()`` accessors) or merged (``merge_results=True``,
    the Rust ``DatabaseSet`` collapse — decision 19)."""

    def __init__(self, db: Any, merge_results: bool = True) -> None:
        self._db = db
        self._merge_results = merge_results

    def __getattr__(self, name: str) -> Any:
        # Reached only when the attribute is not found normally.
        db = self.__dict__["_db"]
        if not self.__dict__["_merge_results"]:
            per_fragment = _PER_FRAGMENT.get(name)
            if per_fragment is not None:
                return getattr(db, per_fragment)
        return getattr(db, name)

    def returncode(self) -> Any:
        """The Rust ``DatabaseSet`` collapses upstream's per-proc
        fan-out and raises directly, so there is no per-proc return
        code to surface — always OK for ``MiliDatabase``'s check."""
        return (ReturnCode.OK, "")

    def clear_return_code(self) -> None:
        """No-op: see :meth:`returncode`."""

    def close(self) -> None:
        """No subprocesses to tear down (fan-out is in Rust)."""


class LoopWrapper(_EngineWrapper):
    """Serial-over-fragments handler identity (suppress_parallel)."""


class ServerWrapper(_EngineWrapper):
    """Parallel handler identity. ``close()`` is the documented
    teardown hook upstream callers invoke."""
