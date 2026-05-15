"""milox ``adjacency`` — the serial ``AdjacencyMapping``.

Upstream ``AdjacencyMapping``
(`reference/mili-python/src/mili/adjacency.py`) is a wrapper around
``MiliDatabase`` exposing mesh-neighbour / connectivity-graph queries.
The value/topology computation is **parity-sensitive**, so it lives in
the Rust core (`mili_rs::adjacency`); this class is the thin
upstream-API-compatible adapter. The upstream ``if not self.serial``
branches (per-proc fan-out) collapse to identity here — the Rust
``DatabaseSet`` already merges — so serial == the core result. Matches
upstream's public surface exactly. See ``planning/mili-py/m4.md``
Phase H.
"""

from __future__ import annotations

from typing import Any, Dict, Tuple

import numpy as np

from .mdg_defines import mdg_enum_to_string


class AdjacencyMapping:
    """Thin adapter over a milox ``MiliDatabase`` (Rust core)."""

    def __init__(self, mili: Any) -> None:
        self.mili = mili
        self.serial = mili.serial

    @property
    def _db(self) -> Any:
        # MiliDatabase -> _MiliInternal -> Rust PyMiliDatabase.
        return self.mili._mili._db

    def compute_centroid(
        self, entity_type: Any, label: int, state: int
    ) -> np.ndarray:
        entity_type_str = mdg_enum_to_string(entity_type)
        centroid = self.mili.geometry.compute_centroid(
            entity_type_str, label, state
        )
        if centroid is None:
            raise ValueError(
                f"Could not calculate centroid for entity_type={entity_type_str}, "
                f"label={label} at state {state}.\n"
                f"Make sure that the specified entity type, label and state all exist."
            )
        return centroid

    def mesh_entities_within_radius(
        self,
        entity_type: Any,
        label: int,
        state: int,
        radius: float,
        material: Any = None,
    ) -> Dict[str, np.ndarray]:
        # Upstream: compute_centroid (raises on None) -> near_coordinate.
        centroid = self.compute_centroid(entity_type, label, state)
        return self.mesh_entities_near_coordinate(
            centroid, state, radius, material
        )

    def mesh_entities_near_coordinate(
        self,
        coordinate: Any,
        state: int,
        radius: float,
        material: Any = None,
    ) -> Dict[str, np.ndarray]:
        return self._db.adj_mesh_entities_near_coordinate(
            list(np.asarray(coordinate, dtype=np.float64)),
            state,
            radius,
            material,
        )

    def elems_of_nodes(
        self, node_labels: Any, material: Any = None
    ) -> Dict[str, np.ndarray]:
        # Serial AdjacencyMapping.elems_of_nodes == geometry.elems_of_nodes.
        return self.mili.geometry.elems_of_nodes(node_labels, material)

    def nearest_node(
        self, point: Any, state: int, material: Any = None
    ) -> Tuple[int, float]:
        return self.mili.geometry.nearest_node(point, state, material)

    def nearest_element(
        self,
        point: Any,
        state: int,
        material: Any = None,
        entity_type: Any = None,
        superclass: Any = None,
    ) -> Tuple[str, int, float]:
        return self.mili.geometry.nearest_element(
            point, state, material, entity_type, superclass
        )

    def neighbor_elements(
        self,
        entity_type: Any,
        label: int,
        material: Any = None,
        neighbor_radius: int = 1,
    ) -> Dict[str, np.ndarray]:
        entity_type_str = mdg_enum_to_string(entity_type)
        code, elements = self._db.adj_neighbor_elements(
            entity_type_str, label, material, neighbor_radius
        )
        if code == 1:
            raise ValueError(
                f"No labels found for entity_type '{entity_type_str}'"
            )
        if code == 2:
            raise ValueError(
                f"The label '{label}' was not found for the entity type "
                f"'{entity_type_str}'"
            )
        return elements

    def neighbor_nodes(self, entity_type: Any, label: int) -> np.ndarray:
        return np.array(
            self._db.adj_neighbor_nodes(
                mdg_enum_to_string(entity_type), label
            ),
            dtype=np.int32,
        )
