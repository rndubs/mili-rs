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
from typing import Any, Dict, List, Optional, Union

import numpy as np
import pandas as pd

from ._native import MiliPythonError
from .datatypes import QueryDict, ReturnCode, ReturnCodeTuple
from .mdg_defines import mdg_enum_to_string
from .miliinternal import _MiliInternal
from .reductions import combine
from .utils import result_dictionary_to_dataframe

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

    def _postprocess_return_codes(self) -> None:
        """Upstream ``MiliDatabase.__postprocess`` return-code half
        (decision 19 — the merge already happened in the Rust core, so
        ``__postprocess`` is identity apart from this raise). Snapshot →
        clear → raise so a raised error never strands a stale code."""
        engine = self.__dict__["_mili"]
        return_codes = engine.returncode()
        engine.clear_return_code()
        parse_return_codes(return_codes)

    def _process_query_modifier(
        self,
        modifier: "ResultModifier",
        results: Union[Dict[str, QueryDict], List[Dict[str, QueryDict]]],
        as_dataframe: bool,
    ) -> Dict[str, QueryDict]:
        """Verbatim port of upstream
        ``MiliDatabase.__process_query_modifier``
        (``reference/mili-python/src/mili/milidatabase.py:619``).

        Decision 18 non-parity post-process: a pure numpy reduction over
        the already-parity-correct primal/derived ``QueryDict`` the Rust
        core returns. ``reductions.combine`` is identity here (the Rust
        ``DatabaseSet`` already merged) — kept to mirror upstream
        exactly, never a Python re-reduce of core data."""
        data: "np.ndarray"
        merged_results = combine(results)
        for svar in merged_results:
            merged_results[svar]["modifier"] = modifier.value

        ##### Minimum #####
        if modifier == ResultModifier.MIN:
            for svar in merged_results:
                labels = merged_results[svar]["layout"]["labels"]
                min_indexes = np.argmin(
                    merged_results[svar]["data"], axis=1, keepdims=True
                )
                merged_results[svar]["data"] = np.take_along_axis(
                    merged_results[svar]["data"], min_indexes, axis=1
                )
                merged_results[svar]["layout"]["labels"] = labels[
                    min_indexes.flatten()
                ]

            if as_dataframe:
                for svar in merged_results:
                    states = merged_results[svar]["layout"]["states"]
                    labels = merged_results[svar]["layout"]["labels"]
                    if len(states) == len(labels):
                        data = merged_results[svar]["data"].flatten()
                    else:
                        data = np.reshape(
                            merged_results[svar]["data"], (len(states), -1)
                        )
                        labels = np.reshape(labels, (len(states), -1))
                    merged_results[svar] = pd.DataFrame(
                        zip(data, labels),
                        index=states,
                        columns=[modifier.value, "label"],
                    )

        ##### Maximum #####
        elif modifier == ResultModifier.MAX:
            for svar in merged_results:
                labels = merged_results[svar]["layout"]["labels"]
                max_indexes = np.argmax(
                    merged_results[svar]["data"], axis=1, keepdims=True
                )
                merged_results[svar]["data"] = np.take_along_axis(
                    merged_results[svar]["data"], max_indexes, axis=1
                )
                merged_results[svar]["layout"]["labels"] = labels[
                    max_indexes.flatten()
                ]

            if as_dataframe:
                for svar in merged_results:
                    states = merged_results[svar]["layout"]["states"]
                    labels = merged_results[svar]["layout"]["labels"]
                    if len(states) == len(labels):
                        data = merged_results[svar]["data"].flatten()
                    else:
                        data = np.reshape(
                            merged_results[svar]["data"], (len(states), -1)
                        )
                        labels = np.reshape(labels, (len(states), -1))
                    merged_results[svar] = pd.DataFrame(
                        zip(data, labels),
                        index=states,
                        columns=[modifier.value, "label"],
                    )

        ##### Average #####
        elif modifier == ResultModifier.AVERAGE:
            for svar in merged_results:
                labels = merged_results[svar]["layout"]["labels"]
                merged_results[svar]["data"] = np.average(
                    merged_results[svar]["data"], axis=1, keepdims=True
                )

            if as_dataframe:
                for svar in merged_results:
                    data = merged_results[svar]["data"]
                    states = merged_results[svar]["layout"]["states"]
                    if data.size != len(states):
                        merged_results[svar] = pd.DataFrame.from_records(
                            data, index=states, columns=[modifier.value]
                        )
                    else:
                        merged_results[svar] = pd.DataFrame(
                            data.flatten(),
                            index=states,
                            columns=[modifier.value],
                        )

        ##### Cumulative Min #####
        elif modifier == ResultModifier.CUMMIN:
            for svar in merged_results:
                states = merged_results[svar]["layout"]["states"]
                for i in range(1, len(states)):
                    merged_results[svar]["data"][i] = np.minimum(
                        merged_results[svar]["data"][i],
                        merged_results[svar]["data"][i - 1],
                    )

        ##### Cumulative Max #####
        elif modifier == ResultModifier.CUMMAX:
            for svar in merged_results:
                states = merged_results[svar]["layout"]["states"]
                for i in range(1, len(states)):
                    merged_results[svar]["data"][i] = np.maximum(
                        merged_results[svar]["data"][i],
                        merged_results[svar]["data"][i - 1],
                    )

        ##### Median #####
        elif modifier == ResultModifier.MEDIAN:
            for svar in merged_results:
                merged_results[svar]["data"] = np.median(
                    merged_results[svar]["data"], axis=1, keepdims=True
                )

            if as_dataframe:
                for svar in merged_results:
                    data = merged_results[svar]["data"].flatten()
                    states = merged_results[svar]["layout"]["states"]
                    if len(data) != len(states):
                        data = merged_results[svar]["data"]
                        merged_results[svar] = pd.DataFrame.from_records(
                            data, index=states, columns=[modifier.value]
                        )
                    else:
                        merged_results[svar] = pd.DataFrame(
                            data, index=states, columns=[modifier.value]
                        )

        ##### Standard Deviation #####
        elif modifier == ResultModifier.STDDEV:
            for svar in merged_results:
                merged_results[svar]["data"] = np.std(
                    merged_results[svar]["data"], axis=1, keepdims=True
                )

            if as_dataframe:
                for svar in merged_results:
                    data = merged_results[svar]["data"].flatten()
                    states = merged_results[svar]["layout"]["states"]
                    if len(data) != len(states):
                        data = merged_results[svar]["data"]
                        merged_results[svar] = pd.DataFrame.from_records(
                            data, index=states, columns=[modifier.value]
                        )
                    else:
                        merged_results[svar] = pd.DataFrame(
                            data, index=states, columns=[modifier.value]
                        )

        return merged_results

    def query(
        self,
        svar_names: Union[List[str], str],
        entity_type: Union[str, Any],
        material: Optional[Union[str, int]] = None,
        labels: Optional[Union[List[int], int]] = None,
        states: Optional[Union[List[int], int]] = None,
        ips: Optional[Union[List[int], int]] = None,
        write_data: Optional[Dict[str, QueryDict]] = None,
        as_dataframe: bool = False,
        modifier: Optional["ResultModifier"] = None,
        project_to_nodes: bool = False,
        **kwargs: Any,
    ) -> Union[Dict[str, "pd.DataFrame"], Dict[str, QueryDict]]:
        """Verbatim port of upstream ``MiliDatabase.query``
        (``milidatabase.py:790``).

        The primal/derived gather is the parity-correct Rust core
        (``self._mili.query`` → ``PyMiliDatabase``); ``__postprocess``
        is identity + return-code raising (decision 19). ``modifier``
        (``ResultModifier`` reductions) and ``as_dataframe``
        (``result_dictionary_to_dataframe``) are decision-18 non-parity
        post-processing ported verbatim over that primal/derived result.
        ``project_to_nodes`` is a still-unported later sub-slice."""
        if write_data and modifier:
            raise ValueError(
                "Result modifiers may not be used when the write_data argument is passed."
            )
        if project_to_nodes and modifier:
            raise ValueError(
                "Result modifiers may not be used when the project_to_nodes flag is True."
            )
        if project_to_nodes and write_data:
            raise ValueError(
                "write_data argument may not be used when the project_to_nodes flag is True."
            )

        result: Union[Dict[str, pd.DataFrame], Dict[str, QueryDict]]

        entity_type_str = mdg_enum_to_string(entity_type)
        if isinstance(svar_names, list):
            svar_names = [mdg_enum_to_string(svar) for svar in svar_names]
        else:
            svar_names = mdg_enum_to_string(svar_names)

        engine = self.__dict__["_mili"]
        # __postprocess: the Rust DatabaseSet already merged, so combine
        # is identity here (decision 19) — kept to mirror upstream.
        result = combine(
            engine.query(
                svar_names,
                entity_type_str,
                material,
                labels,
                states,
                ips,
                write_data,
                **kwargs,
            )
        )
        self._postprocess_return_codes()

        if modifier:
            result = self._process_query_modifier(
                modifier, result, as_dataframe
            )

        if project_to_nodes:
            raise MiliPythonError(
                "project_to_nodes: not yet ported (mili-py M4-followup "
                "phase H projection sub-slice; see planning/mili-py/m4.md "
                "decision 19)"
            )

        if as_dataframe:
            return result_dictionary_to_dataframe(result)
        return result
