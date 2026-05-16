"""milox ``geometric_mesh_info`` — the ``_MiliInternal.geometry``
object.

Upstream ``GeometricMeshInfo``
(`reference/mili-python/src/mili/geometric_mesh_info.py`) computes
nearest-node / nearest-element / centroid / nodes-in-radius /
elems-of-nodes over the primal ``nodpos`` query + connectivity. That
is **parity-sensitive value/topology computation**, so it lives in the
Rust core (`mili_rs::adjacency`); this class is the thin
upstream-API-compatible adapter that delegates to it (decision 19 /
the Phase-H architecture invariant). Matches upstream's public surface
exactly. See ``planning/mili-py/m4.md`` Phase H.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional, Tuple

import numpy as np

from .mdg_defines import mdg_enum_to_string


class GeometricMeshInfo:
    """Thin adapter over a milox ``_MiliInternal`` (Rust core)."""

    def __init__(self, db: Any) -> None:
        self.db = db

    @property
    def _db(self) -> Any:
        # The Rust PyMiliDatabase under the _MiliInternal adapter.
        return self.db._db

    def compute_centroid(
        self, class_name: Any, label: int, state: int
    ) -> Optional[np.ndarray]:
        centroid = self._db.gmi_compute_centroid(
            mdg_enum_to_string(class_name), label, state
        )
        if centroid is None:
            return None
        # Upstream returns the np.sum dtype of the nodpos buffer
        # (float32 for single-precision plt, float64 for double); the
        # core already produced that value — np.array infers it back.
        return np.array(centroid)

    def nearest_node(
        self, point: Any, state: int, material: Any = None
    ) -> Tuple[int, float]:
        return self._db.gmi_nearest_node(point, state, material)

    def nearest_element(
        self,
        point: Any,
        state: int,
        material: Any = None,
        entity_type: Any = None,
        superclass: Any = None,
    ) -> Tuple[str, int, float]:
        et = None if entity_type is None else mdg_enum_to_string(entity_type)
        sc = None if superclass is None else int(superclass)
        return self._db.gmi_nearest_element(point, state, material, et, sc)

    def nodes_within_radius(
        self, center: Any, radius: float, state: int, material: Any = None
    ) -> np.ndarray:
        return np.array(
            self._db.gmi_nodes_within_radius(center, radius, state, material),
            dtype=np.int32,
        )

    def elems_of_nodes(
        self, node_labels: Any, material: Any = None
    ) -> Dict[str, np.ndarray]:
        return self._db.gmi_elems_of_nodes(node_labels, material)
