"""milox ``_MiliInternal`` — the single-fragment engine identity.

Upstream ``_MiliInternal`` parses one process's A/T/S files and answers
~50 read methods. In milox the parsing **and the reshape computation**
live in the Rust core (`PyMiliDatabase` / `mili_rs::reshape`); this
class is the upstream-API-compatible adapter the ``MiliDatabase``
wrapper holds (so ``isinstance(db._mili, _MiliInternal)`` and
``reader.open_database``'s handler selection match upstream), and the
object ``test_miliinternal.py`` constructs directly via
``_MiliInternal(dir_name, base_filename)``.

Phase G adds the primal-only read surface: each method is a thin box
of the Rust core's bit-exact reshape into upstream's
``StateVariable`` / ``Subrecord`` / ``MeshObjectClass`` shapes. Methods
still in ``_UNPORTED`` (Phase H geometry/derived/adjacency) raise a
typed ``MiliPythonError`` naming the phase — never a silent wrong
answer. See ``planning/mili-py/m4.md`` decision 19.
"""

from __future__ import annotations

import os
from typing import Any, Dict, List, Optional

import numpy as np

from ._native import MiliPythonError, PyMiliDatabase
from .afileIO import afiles_by_base
from .datatypes import (
    Metadata,
    MeshObjectClass,
    MiliType,
    ReturnCode,
    StateMap,
    StateVariable,
    Subrecord,
    Superclass,
)

__all__ = [
    "_MiliInternal",
    "ReturnCode",
    "StateVariable",
    "Subrecord",
    "MeshObjectClass",
    "MiliType",
    "Superclass",
    "Metadata",
    "MiliPythonError",
]

# Upstream _MiliInternal read methods not yet on the Rust surface.
# Phase G landed every primal-only reshape; the remainder is Phase H
# (geometry / derived / adjacency — value-producing, fully parity-gated).
_UNPORTED = {
    "nodes_of_elems": "H",
    "nodes_of_material": "H",
    "faces": "H",
    "geometry": "H",
    "connectivity_ids": "H",
    "supported_derived_variables": "H",
    "derived_variables_of_class": "H",
    "classes_of_derived_variable": "H",
}


def _np_i32(values: Any) -> "np.ndarray":
    return np.array(values, dtype=np.int32)


class _MiliInternal:
    """Single-fragment engine identity over a ``PyMiliDatabase``."""

    def __init__(
        self,
        dir_name: Any,
        base_filename: Optional[Any] = None,
        **kwargs: Any,
    ) -> None:
        self._return_code = (ReturnCode.OK, "")
        if base_filename is None and isinstance(dir_name, PyMiliDatabase):
            # reader.open_database path: an already-opened engine.
            self._db = dir_name
        else:
            # Upstream-compatible path (test_miliinternal constructs
            # this directly): discover the A-file(s) under dir_name and
            # open the Rust core. Single fragment -> open_single; a
            # multi-fragment base -> open_set (the Rust DatabaseSet
            # collapses upstream's per-proc fan-out).
            dn = os.fspath(dir_name)
            if dn == "":
                dn = os.getcwd()
            bf = os.fspath(base_filename)
            afiles = afiles_by_base(dn, bf)
            if len(afiles) == 1:
                self._db = PyMiliDatabase.open_single(os.path.join(dn, afiles[0]))
            else:
                self._db = PyMiliDatabase.open_set(os.path.join(dn, bf))

    # ---- return-code plumbing (MiliDatabase.__postprocess) ----
    def returncode(self) -> Any:
        return self._return_code

    def clear_return_code(self) -> None:
        self._return_code = (ReturnCode.OK, "")

    def _err(self, msg: str) -> None:
        self._return_code = (ReturnCode.ERROR, msg)

    def close(self) -> None:
        return None

    # ---- Phase H placeholder ----
    @property
    def geometry(self) -> Any:
        raise MiliPythonError(
            "geometry: not yet ported (mili-py M4-followup phase H; "
            "see planning/mili-py/m4.md decision 19)"
        )

    # ---- Phase G: primal-only reshapes (logic in Rust core) ----
    def metadata(self) -> Metadata:
        return Metadata(**self._db.metadata())

    def reload_state_maps(self) -> bool:
        return self._db.reload_state_maps()

    def superclass_from_class_name(self, class_name: str) -> Superclass:
        code = self._db.superclass_from_class_name(class_name)
        if code == -1:
            self._err(f"The class '{class_name}' does not exist.")
            return Superclass.M_INVALID_LABEL
        return Superclass(code)

    def srec_fmt_qty(self) -> int:
        return self._db.srec_fmt_qty()

    def subrecords(self) -> List[Subrecord]:
        out: List[Subrecord] = []
        for (
            name,
            class_name,
            superclass,
            organization,
            qty_svars,
            svar_names,
            ordinal_blocks,
        ) in self._db.subrecords():
            out.append(
                Subrecord(
                    name=name,
                    class_name=class_name,
                    superclass=Superclass(superclass),
                    organization=Subrecord.Org(organization),
                    qty_svars=qty_svars,
                    svar_names=list(svar_names),
                    ordinal_blocks=np.array(ordinal_blocks, dtype=np.int64),
                )
            )
        return out

    def state_variables(self) -> Dict[str, StateVariable]:
        out: Dict[str, StateVariable] = {}
        for (
            name,
            title,
            data_type,
            agg_type,
            list_size,
            order,
            dims,
            comp_names,
            containing,
        ) in self._db.state_variables_info():
            out[name] = StateVariable(
                name=name,
                title=title,
                agg_type=agg_type,
                data_type=MiliType(data_type),
                list_size=list_size,
                order=order,
                dims=list(dims),
                comp_names=list(comp_names),
                containing_svar_names=list(containing),
            )
        return out

    def queriable_svars(
        self, vector_only: bool = False, show_ips: bool = False
    ) -> List[str]:
        return self._db.queriable_svars(vector_only, show_ips)

    def classes_of_state_variable(self, svar: str) -> List[str]:
        classes, found = self._db.classes_of_state_variable(svar)
        if not found:
            self._err(f"The state variable {svar} does not exists")
        return classes

    def state_variables_of_class(self, class_name: str) -> List[str]:
        svars, found = self._db.state_variables_of_class(class_name)
        if not found:
            self._err(f"The class '{class_name}' does not exist.")
        return svars

    def containing_state_variables_of_class(
        self, svar: str, class_name: str
    ) -> List[str]:
        svars, svar_ok, class_ok = self._db.containing_state_variables_of_class(
            svar, class_name
        )
        if not svar_ok:
            self._err(f"The svar '{svar}' does not exist.")
        elif not class_ok:
            self._err(f"The class '{class_name}' does not exist.")
        return svars

    def components_of_vector_svar(self, svar: str) -> List[str]:
        comps, code = self._db.components_of_vector_svar(svar)
        if code == 1:
            self._err(f"The state variable {svar} does not exists")
        elif code == 2:
            self._err(f"The state variable {svar} is not a Vector")
        return comps

    def state_variable_titles(self) -> Dict[str, str]:
        return self._db.state_variable_titles()

    def int_points_of_state_variable(
        self, svar_name: str, class_name: str
    ) -> "np.ndarray":
        ips, svar_ok, class_ok = self._db.int_points_of_state_variable(
            svar_name, class_name
        )
        if not svar_ok:
            self._err(f"The svar '{svar_name}' does not exist.")
        elif not class_ok:
            self._err(f"The class '{class_name}' does not exist.")
        return _np_i32(ips)

    def mesh_object_classes(self) -> Dict[str, MeshObjectClass]:
        out: Dict[str, MeshObjectClass] = {}
        for (
            short_name,
            mesh_id,
            long_name,
            sclass,
            elem_qty,
            idents_exist,
        ) in self._db.mesh_object_classes():
            out[short_name] = MeshObjectClass(
                mesh_id=mesh_id,
                short_name=short_name,
                long_name=long_name,
                sclass=Superclass(sclass),
                elem_qty=elem_qty,
                idents_exist=idents_exist,
            )
        return out

    def parts_of_class_name(self, class_name: str) -> "np.ndarray":
        parts, class_ok = self._db.parts_of_class_name(class_name)
        if not class_ok:
            self._err(f"The class '{class_name}' does not exist.")
        return _np_i32(parts)

    def materials_of_class_name(self, class_name: str) -> "np.ndarray":
        mats, class_ok = self._db.materials_of_class_name(class_name)
        if not class_ok:
            self._err(f"The class '{class_name}' does not exist.")
        return _np_i32(mats)

    def _valid_material_type(self, mat: Any) -> bool:
        return isinstance(mat, (str, int, np.integer))

    def material_classes(self, mat: Any) -> List[str]:
        if not self._valid_material_type(mat):
            self._err("material must be string or int")
            return []
        if isinstance(mat, np.integer):
            mat = int(mat)
        return self._db.material_classes(mat)

    def class_labels_of_material(
        self, material: Any, class_name: str
    ) -> "np.ndarray":
        if not self._valid_material_type(material):
            self._err("material must be string or int")
            return _np_i32([])
        if isinstance(material, np.integer):
            material = int(material)
        labels, class_ok = self._db.class_labels_of_material(material, class_name)
        if not class_ok:
            self._err(f"The class '{class_name}' does not exist.")
        return _np_i32(labels)

    def all_labels_of_material(self, mat: Any) -> Dict[str, "np.ndarray"]:
        if not self._valid_material_type(mat):
            self._err("material must be string or int")
            return {}
        if isinstance(mat, np.integer):
            mat = int(mat)
        return {
            cls: _np_i32(lbls)
            for cls, lbls in self._db.all_labels_of_material(mat).items()
        }

    def parameters(self) -> Dict[str, Any]:
        return self._db.parameters()

    def parameter(self, name: str, default: Optional[Any] = None) -> Any:
        value = self._db.parameter(name)
        return default if value is None else value

    # ---- already-ported accessors needing upstream arg/return shape ----
    def labels(self, class_name: Optional[str] = None) -> Any:
        if class_name is None:
            return self._db.labels()
        return _np_i32(self._db.labels_of_class(class_name))

    def times(self, states: Optional[Any] = None) -> "np.ndarray":
        all_times = np.array(self._db.times(), dtype=np.float64)
        if states is None:
            return all_times
        states_arr = np.atleast_1d(np.asarray(states, dtype=np.int32))
        return np.array(
            [all_times[s - 1] for s in states_arr], dtype=np.float64
        )

    def state_maps(self) -> List[StateMap]:
        return [
            StateMap(
                file_number=sm["file_number"],
                file_offset=sm["file_offset"],
                time=sm["time"],
            )
            for sm in self._db.state_maps()
        ]

    # ---- forward already-ported accessors / raise for Phase H ----
    def __getattr__(self, name: str) -> Any:
        db = self.__dict__["_db"]
        if hasattr(db, name):
            return getattr(db, name)
        phase = _UNPORTED.get(name)
        if phase is not None:
            raise MiliPythonError(
                f"{name}: not yet ported (mili-py M4-followup phase "
                f"{phase}; see planning/mili-py/m4.md decision 19)"
            )
        raise AttributeError(name)
