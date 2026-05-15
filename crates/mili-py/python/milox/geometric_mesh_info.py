"""milox ``geometric_mesh_info`` — Phase H placeholder.

Upstream ``GeometricMeshInfo`` computes geometry/derived mesh
quantities (faces, areas, centroids, adjacency). That is **Phase H**
(value-producing, fully parity-gated, lands in the Rust core). This
module exists so the read-path suite's
``from mili.geometric_mesh_info import GeometricMeshInfo`` import
resolves and collection succeeds; constructing or using it raises a
typed error naming the phase rather than silently returning wrong
values. See ``planning/mili-py/m4.md`` decision 19.
"""

from __future__ import annotations

from typing import Any

from ._native import MiliPythonError


class GeometricMeshInfo:
    """Phase-H placeholder. Importable (so redirected modules collect);
    not yet functional."""

    def __init__(self, db: Any) -> None:
        raise MiliPythonError(
            "GeometricMeshInfo: not yet ported (mili-py M4-followup "
            "phase H; see planning/mili-py/m4.md decision 19)"
        )
