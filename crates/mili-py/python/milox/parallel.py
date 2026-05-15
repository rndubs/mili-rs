"""milox parallel-handler markers.

Upstream selects one of ``_MiliInternal`` / ``LoopWrapper`` /
``ServerWrapper`` as the per-database engine and fans out across MPI
fragments in Python. In milox the fan-out + merge live in the Rust
``DatabaseSet`` (`PyMiliDatabase.open_set`), so these are thin marker
wrappers over a single already-opened ``PyMiliDatabase``. They exist so
``isinstance(db._mili, LoopWrapper)`` and the handler-selection logic
in ``reader.open_database`` stay byte-compatible with upstream.

Attribute access forwards to the wrapped engine; the
parity-sensitive logic stays in Rust (m1–m3 precedent).
"""

from __future__ import annotations

from typing import Any


class _EngineWrapper:
    """Base: hold a ``PyMiliDatabase`` and forward to it."""

    def __init__(self, db: Any) -> None:
        self._db = db

    def __getattr__(self, name: str) -> Any:
        # Only reached when the attribute is not found normally.
        return getattr(self.__dict__["_db"], name)

    def close(self) -> None:
        """No subprocesses to tear down (fan-out is in Rust)."""


class LoopWrapper(_EngineWrapper):
    """Serial-over-fragments handler identity (suppress_parallel)."""


class ServerWrapper(_EngineWrapper):
    """Parallel handler identity. ``close()`` is the documented
    teardown hook upstream callers invoke."""
