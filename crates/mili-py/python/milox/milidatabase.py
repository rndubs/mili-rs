"""milox ``MiliDatabase`` wrapper.

The user-facing handle, byte-compatible with upstream
``mili.milidatabase.MiliDatabase`` at the level the read-path suite
touches: it holds ``_mili`` (one of the ``parallel`` / ``miliinternal``
engine identities over a Rust ``PyMiliDatabase``), exposes
``serial`` / ``merge_results`` / ``close`` / context-manager, and
forwards every read accessor to the engine.

``__postprocess`` collapses to identity here apart from return-code
raising: the Rust ``DatabaseSet`` already performs upstream's
per-fragment merge (``reductions.*``) before results cross the FFI
boundary, so the Python wrapper must not re-reduce. This matches
upstream exactly for the serial / single-fragment case and is correct
for the merged set case because the merge already happened in core
(m1–m3 precedent). What the wrapper *does* keep from upstream
``__postprocess`` is the return-code check: each forwarded accessor
runs ``engine.returncode()`` -> ``parse_return_codes`` (which raises
``MiliPythonError``) -> ``engine.clear_return_code()``, and any
``EntityType`` / ``StateVariableName`` argument is coerced through
``mdg_enum_to_string`` first (identity for ``str`` / ``int`` /
``list``), matching upstream's per-method coercion.
"""

from __future__ import annotations

from enum import Enum
from typing import Any, List, Union

from ._native import MiliPythonError
from .datatypes import ReturnCode, ReturnCodeTuple
from .mdg_defines import mdg_enum_to_string
from .miliinternal import _MiliInternal

__all__ = ["MiliDatabase", "MiliPythonError", "ResultModifier", "parse_return_codes"]


def parse_return_codes(
    return_codes: Union[ReturnCodeTuple, List[ReturnCodeTuple]],
) -> None:
    """Processes return codes from MiliDatabase and check for errors or exceptions.

    Verbatim port of upstream ``mili.milidatabase.parse_return_codes``.
    """
    if isinstance(return_codes, tuple):
        return_codes = [return_codes]
    if not all([rcode_tup[0] == ReturnCode.OK for rcode_tup in return_codes]):
        # An error has occurred. Need to determine severity.
        num_rc = len(return_codes)
        errors = [rc for rc in return_codes if rc[0] == ReturnCode.ERROR]
        critical = [rc for rc in return_codes if rc[0] == ReturnCode.CRITICAL]
        if len(critical) > 0 or len(errors) == num_rc:
            error_msgs = list(
                set(
                    [
                        f"{rc[0].str_repr()}: {rc[1]}"
                        for rc in return_codes
                        if rc[0] in (ReturnCode.ERROR, ReturnCode.CRITICAL)
                    ]
                )
            )
            raise MiliPythonError(", ".join(error_msgs))


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
        engine = self.__dict__["_mili"]
        attr = getattr(engine, name)
        if not callable(attr):
            return attr

        def _forward(*args: Any, **kwargs: Any) -> Any:
            # Coerce any EntityType / StateVariableName argument to its
            # string value (mdg_enum_to_string is identity for
            # str / int / list), matching upstream's per-method
            # mdg_enum_to_string calls.
            args = tuple(mdg_enum_to_string(a) for a in args)
            kwargs = {k: mdg_enum_to_string(v) for k, v in kwargs.items()}
            result = attr(*args, **kwargs)
            # Upstream MiliDatabase.__postprocess order: snapshot the
            # return code, clear it, *then* raise — so a raised error
            # never leaves a stale code that re-raises on the next
            # call. The Rust DatabaseSet already merged per-fragment
            # results, so there is no Python-side reduce (decision 19).
            return_codes = engine.returncode()
            engine.clear_return_code()
            parse_return_codes(return_codes)
            return result

        return _forward
