"""milox ``_MiliInternal`` adapter.

Upstream ``_MiliInternal`` parses one process's A/T/S files and answers
~50 read methods. In milox that parsing + the parity-sensitive read
path live in the Rust core (`PyMiliDatabase`); this class is the
single-fragment **engine identity** the ``MiliDatabase`` wrapper holds
and that ``reader.open_database`` selects for serial / 1-fragment /
proc-subset databases (so ``isinstance(db._mili, _MiliInternal)``
matches upstream).

Attribute access forwards to the wrapped ``PyMiliDatabase``. Methods
that exist upstream but are not yet on the Rust surface raise a typed
``MiliPythonError`` naming the M4-followup phase — never a silent
wrong answer (see ``planning/mili-py/m4.md`` decision 19).
"""

from __future__ import annotations

from typing import Any

from ._native import MiliPythonError
from .datatypes import ReturnCode

# Upstream _MiliInternal read methods not yet on the Rust surface.
# Phase G = primal-only reshapes; Phase H = geometry/derived/adjacency.
_UNPORTED = {
    "subrecords": "G",
    "state_variables": "G",
    "queriable_svars": "G",
    "classes_of_state_variable": "G",
    "containing_state_variables_of_class": "G",
    "class_labels_of_material": "G",
    "all_labels_of_material": "G",
    "parts_of_class_name": "G",
    "materials_of_class_name": "G",
    "state_variable_titles": "G",
    "components_of_vector_svar": "G",
    "int_points_of_state_variable": "G",
    "mesh_object_classes": "G",
    "superclass_from_class_name": "G",
    "material_classes": "G",
    "state_variables_of_class": "G",
    "parameters": "G",
    "parameter": "G",
    "srec_fmt_qty": "G",
    "nodes_of_elems": "H",
    "nodes_of_material": "H",
    "faces": "H",
    "geometry": "H",
    "connectivity_ids": "H",
    "supported_derived_variables": "H",
    "derived_variables_of_class": "H",
    "classes_of_derived_variable": "H",
}


class _MiliInternal:
    """Single-fragment engine identity over a ``PyMiliDatabase``."""

    def __init__(self, db: Any) -> None:
        self._db = db

    # ---- MiliDatabase.__postprocess plumbing (serial → identity) ----
    def returncode(self) -> Any:
        return (ReturnCode.OK, "")

    def clear_return_code(self) -> None:
        return None

    def close(self) -> None:
        return None

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
