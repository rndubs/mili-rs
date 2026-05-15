"""milox ``MiliDatabase`` wrapper.

The user-facing handle, byte-compatible with upstream
``mili.milidatabase.MiliDatabase`` at the level the read-path suite
touches: it holds ``_mili`` (one of the ``parallel`` / ``miliinternal``
engine identities over a Rust ``PyMiliDatabase``), exposes
``serial`` / ``merge_results`` / ``close`` / context-manager, and
forwards every read accessor to the engine.

``__postprocess`` collapses to identity here: the Rust ``DatabaseSet``
already performs upstream's per-fragment merge (``reductions.*``)
before results cross the FFI boundary, so the Python wrapper must not
re-reduce. This matches upstream exactly for the serial / single-
fragment case and is correct for the merged set case because the merge
already happened in core (m1–m3 precedent).
"""

from __future__ import annotations

from enum import Enum
from typing import Any

from ._native import MiliPythonError
from .miliinternal import _MiliInternal

__all__ = ["MiliDatabase", "MiliPythonError", "ResultModifier"]


class ResultModifier(Enum):
    """Verbatim port of upstream ``mili.milidatabase.ResultModifier``
    (the query-result reduction selector). Pure enum, zero parity
    risk."""

    CUMMIN = "cummin"
    CUMMAX = "cummax"
    MIN = "min"
    MAX = "max"
    AVERAGE = "average"
    MEDIAN = "median"
    STDDEV = "stddev"


class MiliDatabase:
    def __init__(self, mili_engine: Any, merge_results: bool = True) -> None:
        self._mili = mili_engine
        self.merge_results = merge_results

    @property
    def serial(self) -> bool:
        """True when a single mili database file backs this handle."""
        return isinstance(self._mili, _MiliInternal)

    def close(self) -> None:
        """Close the database / shut down any subprocesses."""
        close = getattr(self._mili, "close", None)
        if callable(close):
            close()

    def __enter__(self) -> "MiliDatabase":
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        self.close()

    def __getattr__(self, name: str) -> Any:
        # Reached only when the attribute is not defined on the wrapper
        # itself — forward the full read accessor surface to the engine
        # (which forwards to the Rust PyMiliDatabase).
        return getattr(self.__dict__["_mili"], name)
