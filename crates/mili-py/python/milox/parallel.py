"""milox parallel-handler wrappers — faithful per-proc forwarding.

Upstream (`reference/mili-python/src/mili/parallel.py`) selects
``LoopWrapper`` (serial-over-fragments, ``suppress_parallel``) or
``ServerWrapper`` (subprocess-per-proc) as the per-database engine.
Both wrap a **list of per-proc ``_MiliInternal``** and forward every
public method as ``[proc.method(*a, **kw) for proc in procs]`` — the
per-proc *unmerged* list. ``MiliDatabase`` then applies the
per-accessor ``reduce_function`` when ``merge_results=True`` and
returns the raw per-proc list when ``False``.

**Phase I.4 decision 21 (recorded — the I.4 architecture point).**
I.4 adopts the upstream contract verbatim: the wrapper holds a real
per-proc list of milox ``_MiliInternal``, each opening **one
fragment's A-file** via ``open_single`` (already serial-gate
bit-exact), and forwards every callable as the per-proc list (the
``geometry`` property rewrapped as a per-proc geometry sub-wrapper,
mirroring upstream's ``LoopWrapper`` property rewrap). The
``merge_results=True`` reduction moves to ``MiliDatabase``'s
per-method ``reduce_function`` table over ``milox.reductions`` (the
existing verbatim port). This **supersedes** the Phase-I.3
``_MiliInternal``-over-Set mechanism for the wrapper path: the
per-proc list + verbatim ``reductions.*`` *is* upstream's exact
algorithm over per-fragment engines that are each individually
serial-gate bit-exact, so it is bit-exact by construction wherever
upstream is — including the ``db0()``-only accessors the Rust
``DatabaseSet`` merge could not reproduce (I.3's honest-xfail
boundary). Decision 19 invariant intact: Phase I adds **no new value
math** (per-proc *list assembly* + the verbatim ``reductions.py`` /
``adjacency.py`` merges are non-parity plumbing over already-parity-
correct per-fragment ``Database`` outputs — decision 20). The Rust
``DatabaseSet`` merge is unchanged and still backs the direct
``_MiliInternal(set_db)`` consumers (e.g. ``test_miliinternal``
opening a parallel base).

``ServerWrapper`` is a single-process identity (no real subprocess
spawner) — same behavior as ``LoopWrapper``; genuinely
subprocess-only semantics (``use_shared_memory``) have no serial
oracle and stay honestly xfailed with a concrete reason.
"""

from __future__ import annotations

from typing import Any, List


class _GeometryWrapper:
    """Per-proc rewrap of the ``_MiliInternal.geometry`` property.

    Upstream's ``LoopWrapper`` rewraps each property as a
    ``LoopWrapper`` over the per-proc property objects; a call then
    fans out to ``[obj.geometry.method(...) for obj in procs]``. milox
    mirrors that exactly so ``MiliDatabase.geometry.compute_centroid``
    et al. yield the per-proc list the verbatim ``adjacency.py`` /
    ``test_adjacency`` parallel paths consume."""

    def __init__(self, geoms: List[Any]) -> None:
        self._geoms = geoms

    def __getattr__(self, name: str) -> Any:
        geoms = self.__dict__["_geoms"]

        def _forward(*args: Any, **kwargs: Any) -> Any:
            try:
                return [getattr(g, name)(*args, **kwargs) for g in geoms]
            except Exception as e:  # noqa: BLE001 — upstream __loop_caller
                return [e]

        return _forward


class _EngineWrapper:
    """Hold the per-proc ``_MiliInternal`` list and forward public
    reads as the upstream per-proc list (decision 21)."""

    def __init__(
        self,
        internal_cls: Any,
        dir_name: Any,
        proc_bases: List[Any],
        merge_results: bool = True,
        **kwargs: Any,
    ) -> None:
        self._procs = [
            internal_cls(dir_name, base, **kwargs) for base in proc_bases
        ]
        self._merge_results = merge_results

    @property
    def geometry(self) -> _GeometryWrapper:
        return _GeometryWrapper([p.geometry for p in self._procs])

    def __getattr__(self, name: str) -> Any:
        # Reached only when the attribute is not found normally — the
        # full read accessor surface fans out per proc (upstream
        # LoopWrapper.__loop_caller).
        procs = self.__dict__["_procs"]

        def _forward(*args: Any, **kwargs: Any) -> Any:
            try:
                return [getattr(p, name)(*args, **kwargs) for p in procs]
            except Exception as e:  # noqa: BLE001 — upstream __loop_caller
                return [e]

        return _forward

    def returncode(self) -> Any:
        """Per-proc return codes (a list of ``(code, msg)``).
        ``MiliDatabase`` feeds the list to ``parse_return_codes``,
        which raises only when *all* procs error or any is CRITICAL —
        exactly upstream's behavior for a per-proc ``_MiliInternal``
        that declares a class/svar on only some ranks."""
        return [p.returncode() for p in self._procs]

    def clear_return_code(self) -> None:
        for p in self._procs:
            p.clear_return_code()

    def close(self) -> None:
        for p in self._procs:
            close = getattr(p, "close", None)
            if callable(close):
                close()


class LoopWrapper(_EngineWrapper):
    """Serial-over-fragments handler identity (suppress_parallel)."""


class ServerWrapper(_EngineWrapper):
    """Parallel handler identity (single-process; ``close()`` is the
    documented teardown hook upstream callers invoke)."""


__all__ = ["LoopWrapper", "ServerWrapper"]
