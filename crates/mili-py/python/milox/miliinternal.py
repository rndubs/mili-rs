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
# Phase H landed the derived-variable *listing* surface
# (supported_derived_variables / derived_variables_of_class /
# classes_of_derived_variable — verbatim metadata over already-ported
# core accessors). The derived *value* engine (stress/strain
# invariants, velocities, accelerations, …) is the next sub-slice and
# routes through milox.derived's explicit typed-error stub /
# the Rust core (node displacement already landed there).
_UNPORTED: Dict[str, str] = {}


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

        # Derived-variable listing engine (Phase H listing sub-slice).
        # Upstream holds `self.__derived = DerivedExpressions(self)`
        # (miliinternal.py:286) and the three listing methods delegate.
        from .derived import DerivedExpressions

        self.__derived = DerivedExpressions(self)

    # ---- Phase H: derived-variable listing (verbatim metadata) ----
    def supported_derived_variables(self) -> List[str]:
        return self.__derived.supported_variables()

    def derived_variables_of_class(self, class_name: str) -> List[str]:
        if class_name not in self.mesh_object_classes():
            self._err(f"The class '{class_name}' does not exist.")
            return []
        return self.__derived.derived_variables_of_class(class_name)

    def classes_of_derived_variable(self, var_name: str) -> List[str]:
        if var_name not in self.__derived.supported_variables():
            self._err(f"The derived variable '{var_name}' does not exist.")
            return []
        return self.__derived.classes_of_derived_variable(var_name)

    # ---- return-code plumbing (MiliDatabase.__postprocess) ----
    def returncode(self) -> Any:
        return self._return_code

    def clear_return_code(self) -> None:
        self._return_code = (ReturnCode.OK, "")

    def _err(self, msg: str) -> None:
        self._return_code = (ReturnCode.ERROR, msg)

    def close(self) -> None:
        return None

    # ---- Phase H: GeometricMeshInfo (Rust-core-backed adapter) ----
    @property
    def geometry(self) -> Any:
        from .geometric_mesh_info import GeometricMeshInfo

        cached = self.__dict__.get("_geometry")
        if cached is None:
            cached = GeometricMeshInfo(self)
            self.__dict__["_geometry"] = cached
        return cached

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

    def material_numbers(self) -> "np.ndarray":
        # Upstream _MiliInternal.material_numbers (miliinternal.py:595)
        # returns np.array(...) — an ndarray, not the Rust core's list.
        # The wrapper's reduce_function (list_concatenate_unique) also
        # yields an ndarray, so the serial/parallel type-equality
        # assertions hold.
        return np.array(self._db.material_numbers())

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

    def connectivity(self, class_name: Optional[str] = None) -> Any:
        # Upstream _MiliInternal.connectivity: with a class name, an
        # unknown class sets the ERROR return code (and still returns
        # an empty array); None returns the per-class dict.
        if class_name is None:
            return self._db.connectivity()
        if self._db.superclass_from_class_name(class_name) == -1:
            self._err(f"The class '{class_name}' does not exist.")
        return self._db.connectivity(class_name)

    def connectivity_ids(self, class_name: Optional[str] = None) -> Any:
        # Upstream _MiliInternal.connectivity_ids
        # (miliinternal.py:631-647): unknown class -> ERROR return code
        # (still returns an empty array); None -> the per-class dict.
        if class_name is None:
            return self._db.connectivity_ids()
        if self._db.superclass_from_class_name(class_name) == -1:
            self._err(f"The class '{class_name}' does not exist.")
        return self._db.connectivity_ids(class_name)

    def nodes_of_elems(
        self, class_sname: str, elem_labels: Any
    ) -> Any:
        # miliinternal.py:920-953. argument_to_ndarray(None) -> None.
        if elem_labels is None:
            self._err("The provided labels are None.")
            return (
                np.empty([1, 0], dtype=np.int32),
                np.empty([1, 0], dtype=np.int32),
            )
        nodes, elems, code = self._db.nodes_of_elems(class_sname, elem_labels)
        if code == 1:
            self._err(f"The class '{class_sname}' does not exist.")
        elif code == 2:
            self._err(
                f"None of the provided labels exist for class '{class_sname}'."
            )
        elif code == 3:
            self._err(
                f"The class '{class_sname}' does not have element connectivity."
            )
        if code != 0:
            return (
                np.empty([1, 0], dtype=np.int32),
                np.empty([1, 0], dtype=np.int32),
            )
        return nodes, elems

    def faces(self, class_name: str, label: int) -> Dict[int, "np.ndarray"]:
        # miliinternal.py:649-685. HEX-only.
        code, flat = self._db.faces(class_name, label)
        if code == 1:
            self._err(f"The element class ({class_name}) does not exist.")
            return {}
        if code == 2:
            self._err("This function only supports HEX element classes.")
            return {}
        if code == 3:
            self._err(
                f"The label ({label}) does not exist for the class "
                f"({class_name})"
            )
            return {}
        arr = _np_i32(flat).reshape(6, 4)
        return {i + 1: arr[i] for i in range(6)}

    def nodes_of_material(self, mat: Any) -> "np.ndarray":
        # miliinternal.py:955-971.
        if not self._valid_material_type(mat):
            self._err("material must be string or int")
            return _np_i32([])
        if isinstance(mat, np.integer):
            mat = int(mat)
        return _np_i32(self._db.nodes_of_material(mat))

    def measure(
        self,
        a_entity_type: Any,
        a_label: int,
        b_entity_type: Any,
        b_label: int,
        states: Optional[Any] = None,
    ) -> Any:
        # MiliDatabase.measure (milidatabase.py:882-923) — upstream
        # places it on the wrapper; milox forwards engine attrs, so it
        # lives here over the self-contained Rust-core centroid geometry.
        distance, a_states = self._db.measure(
            a_entity_type, a_label, b_entity_type, b_label, states
        )
        return np.array(distance, dtype=np.float32), _np_i32(a_states)

    def query(
        self,
        svar_names: Any,
        class_sname: Any,
        material: Any = None,
        labels: Any = None,
        states: Any = None,
        ips: Any = None,
        write_data: Any = None,
        **kwargs: Any,
    ) -> Any:
        # Signature mirrors upstream _MiliInternal.query so Python
        # argument binding raises TypeError for a missing class /
        # unexpected keyword exactly as upstream does. Upstream then
        # validates argument *types* via the ERROR return code (raised
        # by MiliDatabase.__postprocess); the Rust core raises
        # MiliPythonError for the same conditions, so mirror that for
        # the two cases PyO3 would otherwise surface as a bare
        # TypeError (a non-str class / a non-iterable svar).
        if not isinstance(class_sname, str):
            raise MiliPythonError(
                f"The class '{class_sname}' does not exist."
            )
        if not isinstance(svar_names, str):
            try:
                iter(svar_names)
            except TypeError:
                raise MiliPythonError(
                    "State variable names must be a string or iterable "
                    "of strings"
                ) from None
        try:
            return self._db.query(
                svar_names, class_sname, material, labels, states, ips,
                write_data, **kwargs,
            )
        except MiliPythonError as e:
            # Upstream _MiliInternal.query never raises: every failure
            # (svar/class absent on this fragment, no labels, bad
            # states/ips, …) sets the ERROR return code and returns a
            # well-formed but *empty* result dict keyed by the queried
            # svars (miliinternal.py:__query — `return res`). For a
            # parallel db a fragment routinely lacks a class/svar, so
            # this leniency is what lets the LoopWrapper/ServerWrapper
            # build a clean per-proc list (parse_return_codes then
            # raises only when *all* procs error / the serial case),
            # rather than the Rust exception aborting the whole fan-out.
            self._err(str(e))
            from .datatypes import QueryDict, QueryLayout

            if isinstance(svar_names, str):
                names = [svar_names]
            else:
                names = list(svar_names)
            return {
                name: QueryDict(
                    data=np.empty([0], dtype=np.float32),
                    layout=QueryLayout(
                        states=np.empty([0], dtype=np.int32),
                        labels=np.empty([0], dtype=np.int32),
                        components=[],
                        times=np.empty([0], dtype=np.float32),
                    ),
                    source="",
                    class_name=class_sname,
                    title="",
                    modifier="",
                )
                for name in names
            }

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
