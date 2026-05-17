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

import warnings
from enum import Enum
from typing import Any, Dict, List, Optional, Union

import numpy as np
import pandas as pd

from ._native import MiliPythonError
from .datatypes import QueryDict, ReturnCode, ReturnCodeTuple, Superclass
from .mdg_defines import mdg_enum_to_string
from .miliinternal import _MiliInternal
from .projection import (
    beam_to_nodal,
    hex_to_nodal,
    particle_to_nodal,
    quad_to_nodal,
    tet_to_nodal,
    tri_to_nodal,
    truss_to_nodal,
)
from . import reductions
from .reductions import combine
from .utils import result_dictionary_to_dataframe

__all__ = ["MiliDatabase", "MiliPythonError", "ResultModifier", "parse_return_codes"]


# Upstream ``MiliDatabase``'s per-accessor ``reduce_function`` table
# (``reference/mili-python/src/mili/milidatabase.py`` — each named
# method calls ``__postprocess(self._mili.X(...), reduce_function=...)``).
# Phase I.4 decision 21, option (b): milox keeps its generic
# ``__getattr__`` forwarder and drives the per-method merge from this
# name→reducer map over the verbatim ``milox.reductions`` port. A
# method absent from the table is *not* a postprocessed ``MiliDatabase``
# accessor upstream (it is read raw via ``db._mili.X()`` — e.g.
# ``connectivity_ids`` / ``mesh_object_classes`` / ``subrecords`` /
# ``parameters`` from ``grizinterface``), so it returns the raw per-proc
# list under a wrapper exactly as upstream. ``nodes`` / ``query`` /
# ``measure`` have bespoke handling (explicit methods below).
_REDUCE_FUNCTIONS: Dict[str, Any] = {
    "reload_state_maps": reductions.zeroth_entry,
    "metadata": reductions.zeroth_entry,
    "superclass_from_class_name": reductions.reduce_superclass_from_class_names,
    "state_maps": reductions.zeroth_entry,
    "srec_fmt_qty": reductions.zeroth_entry,
    "mesh_dimensions": reductions.zeroth_entry,
    "state_count": reductions.zeroth_entry,
    "class_names": reductions.list_concatenate_unique_str,
    "int_points_of_state_variable": reductions.list_concatenate_unique,
    "element_sets": reductions.dictionary_merge_no_concat,
    "integration_points": reductions.dictionary_merge_no_concat,
    "times": reductions.zeroth_entry,
    "queriable_svars": reductions.list_concatenate_unique_str,
    "supported_derived_variables": reductions.zeroth_entry,
    "derived_variables_of_class": reductions.list_concatenate_unique_str,
    "classes_of_derived_variable": reductions.list_concatenate_unique_str,
    "labels": reductions.reduce_labels,
    "materials": reductions.zeroth_entry,
    "material_numbers": reductions.list_concatenate_unique,
    "connectivity": reductions.reduce_connectivity,
    "faces": reductions.dictionary_merge_no_concat,
    "material_classes": reductions.list_concatenate_unique_str,
    "classes_of_state_variable": reductions.list_concatenate_unique_str,
    "state_variables_of_class": reductions.list_concatenate_unique_str,
    "state_variable_titles": reductions.dictionary_merge_no_concat,
    "containing_state_variables_of_class": reductions.list_concatenate_unique_str,
    "components_of_vector_svar": reductions.list_concatenate_unique_str,
    "parts_of_class_name": reductions.list_concatenate,
    "materials_of_class_name": reductions.list_concatenate,
    "class_labels_of_material": reductions.list_concatenate,
    "all_labels_of_material": reductions.dictionary_merge_concat,
    "nodes_of_elems": reductions.reduce_nodes_of_elems,
    "nodes_of_material": reductions.list_concatenate_unique,
    "append_state": reductions.zeroth_entry,
    "copy_non_state_data": reductions.zeroth_entry,
}


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
            results = attr(*args, **kwargs)
            # Upstream MiliDatabase.__postprocess order: snapshot the
            # return code, clear it, raise (so a raised error never
            # strands a stale code), then re-raise any per-proc
            # exception the wrapper captured, then apply the
            # per-accessor reduce when merge_results=True (decision 21).
            return_codes = engine.returncode()
            engine.clear_return_code()
            parse_return_codes(return_codes)
            self._check_for_exceptions(results)
            if self.serial or not self.merge_results:
                return results
            reduce_function = _REDUCE_FUNCTIONS.get(name)
            if reduce_function is None:
                return results
            return reduce_function(results)

        return _forward

    def _check_for_exceptions(self, results: Any) -> None:
        """Verbatim port of upstream
        ``MiliDatabase.__check_for_exceptions``: a serial engine yields
        a single result (raise it if it is an Exception); a wrapper
        yields a per-proc list (raise the first Exception the
        ``LoopWrapper``/``ServerWrapper`` ``__loop_caller`` captured)."""
        if self.serial and isinstance(results, Exception):
            raise results
        if not self.serial and isinstance(results, list):
            for res in results:
                if isinstance(res, Exception):
                    raise res

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

    def _project_result(
        self,
        result: Union[Dict[str, QueryDict], List[Dict[str, QueryDict]]],
    ) -> Dict[str, QueryDict]:
        """Verbatim port of upstream ``MiliDatabase.__project_result``
        (``reference/mili-python/src/mili/milidatabase.py:731``).

        Decision-18/19 non-parity post-process: dispatch on the
        already-parity-correct ``superclass_from_class_name``
        (``mili_rs::reshape``) into the ``milox.projection`` routines,
        which are verbatim numpy over the parity-correct core query /
        ``nodes_of_elems`` / ``element_volume``. ``combine`` is identity
        here (the Rust ``DatabaseSet`` already merged — decision 19)."""
        nodal_result: Dict[str, QueryDict] = {}
        result = combine(result)

        warnings.warn(
            "Nodal values are computed by averaging the adjacent element results. This is an "
            "approximation and may introduce error into the field values. Please exercise caution "
            "when using these results for analysis work.",
            UserWarning,
            stacklevel=3,
        )

        for result_name, result_dict in result.items():
            class_name = result_dict["class_name"]
            superclass = self.superclass_from_class_name(class_name)

            if superclass == Superclass.M_NODE:
                nodal_result[result_name] = result_dict
            elif superclass == Superclass.M_HEX:
                nodal_result[result_name] = hex_to_nodal(self, result_dict)
            elif superclass == Superclass.M_QUAD:
                nodal_result[result_name] = quad_to_nodal(self, result_dict)
            elif superclass == Superclass.M_TRI:
                nodal_result[result_name] = tri_to_nodal(self, result_dict)
            elif superclass == Superclass.M_BEAM:
                nodal_result[result_name] = beam_to_nodal(self, result_dict)
            elif superclass == Superclass.M_TRUSS:
                nodal_result[result_name] = truss_to_nodal(self, result_dict)
            elif superclass == Superclass.M_TET:
                nodal_result[result_name] = tet_to_nodal(self, result_dict)
            elif superclass == Superclass.M_PARTICLE or superclass == Superclass.M_INODE:
                nodal_result[result_name] = particle_to_nodal(self, result_dict)
            else:
                raise NotImplementedError(
                    f"Projection is not supported for the type {superclass}"
                )

        return nodal_result

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
        # Upstream MiliDatabase.query: __postprocess(self._mili.query(...),
        # reduce_function=reductions.combine). __postprocess returns the
        # raw per-proc list when (serial or not merge_results) and
        # combine(list) otherwise. The Rust per-fragment query is
        # parity-correct primal/derived; combine / the modifier /
        # projection layers are the decision-18 non-parity post-process.
        results = engine.query(
            svar_names,
            entity_type_str,
            material,
            labels,
            states,
            ips,
            write_data,
            **kwargs,
        )
        return_codes = engine.returncode()
        engine.clear_return_code()
        parse_return_codes(return_codes)
        self._check_for_exceptions(results)
        if self.serial or not self.merge_results:
            result = results
        else:
            result = combine(results)

        if modifier:
            result = self._process_query_modifier(
                modifier, result, as_dataframe
            )

        if project_to_nodes:
            result = self._project_result(result)

        if as_dataframe:
            return result_dictionary_to_dataframe(result)
        return result

    def nodes(self) -> "np.ndarray":
        """Verbatim port of upstream ``MiliDatabase.nodes``
        (``milidatabase.py:168``).

        Bespoke (not a ``__postprocess`` accessor): the
        ``merge_results=True`` merge dedups node coordinates by the
        first appearance of each (duplicated) node label across procs.
        Reads the raw per-proc ``labels("node")`` / ``nodes()`` lists
        from the engine directly, exactly as upstream."""
        engine = self.__dict__["_mili"]
        if self.serial or not self.merge_results:
            return engine.nodes()
        # Concatenate node labels (contains duplicates across procs).
        nlabels = reductions.list_concatenate(engine.labels("node"))
        # Index of first appearance of each node, original order.
        _, indexes = np.unique(nlabels, axis=0, return_index=True)
        indexes.sort()
        nodes = np.concatenate(engine.nodes())
        return nodes[indexes]

    def measure(
        self,
        a_entity_type: Union[str, Any],
        a_label: int,
        b_entity_type: Union[str, Any],
        b_label: int,
        states: Optional[Union[List[int], int]] = None,
    ) -> Any:
        """Verbatim port of upstream ``MiliDatabase.measure``
        (``milidatabase.py:882``).

        Bespoke (not a ``__postprocess`` accessor): distance between
        two elements' centroids over the parity-correct ``centroid``
        derived query; ``reductions.combine`` collapses the per-proc
        list (identity for the serial / merged case)."""
        a_centroid = combine(
            self.query("centroid", a_entity_type, labels=[a_label], states=states)
        )
        a_states = a_centroid["centroid"]["layout"]["states"]
        a_data = a_centroid["centroid"]["data"]

        b_centroid = combine(
            self.query("centroid", b_entity_type, labels=[b_label], states=states)
        )
        b_data = b_centroid["centroid"]["data"]

        x_dist = b_data[:, :, 0] - a_data[:, :, 0]
        y_dist = b_data[:, :, 1] - a_data[:, :, 1]
        z_dist = b_data[:, :, 2] - a_data[:, :, 2]

        x_dist = x_dist * x_dist
        y_dist = y_dist * y_dist
        z_dist = z_dist * z_dist

        distance = np.sqrt(x_dist + y_dist + z_dist).ravel()
        return distance, a_states
