"""milox datatypes — the pure, zero-parity-risk subset of upstream
``mili.datatypes`` that the read-path test suite imports.

``Superclass``, ``Metadata`` and ``ReturnCode`` are copied **verbatim**
from ``reference/mili-python/src/mili/datatypes.py`` (only the unused
heavy dataclasses / AFile parsing scaffolding is omitted — the Rust
core owns A-file parsing). Keeping them byte-identical means
``from mili.datatypes import Superclass, Metadata`` ports unchanged.
"""

from __future__ import annotations

from enum import Enum, IntEnum
from typing import Tuple

import numpy as np
from numpy.typing import NDArray
from typing_extensions import TypeAlias, TypedDict


class Superclass(IntEnum):
    """The superclass denotes what mesh class an object belongs to."""

    M_UNIT = 0
    M_NODE = 1
    M_TRUSS = 2
    M_BEAM = 3
    M_TRI = 4
    M_QUAD = 5
    M_TET = 6
    M_PYRAMID = 7
    M_WEDGE = 8
    M_HEX = 9
    M_MAT = 10
    M_MESH = 11
    M_SURFACE = 12
    M_PARTICLE = 13
    M_TET10 = 14
    M_INODE = 15
    M_QTY_SUPERCLASS = 16
    M_SHARED = 100
    M_ALL = 200
    M_INVALID_LABEL = -1

    def node_count(self) -> int:
        """Return the node count for each superclass."""
        return [0, 0, 2, 3, 3, 4, 4, 5, 6, 8, 0, 0, 0, 1, 10, 1][self.value]

    def node_connections(self) -> NDArray[np.int32]:
        """Return the nodal connections map for each superclass."""
        node_connections = {
            Superclass.M_TRUSS: np.array([[1], [0]]),
            Superclass.M_BEAM: np.array([[1], [0]]),
            Superclass.M_TRI: np.array([[1, 2], [0, 2], [0, 1]]),
            Superclass.M_QUAD: np.array([[1, 3], [0, 2], [1, 3], [0, 2]]),
            Superclass.M_TET: np.array(
                [[1, 2, 3], [0, 2, 3], [0, 1, 3], [0, 1, 2]]
            ),
            Superclass.M_HEX: np.array(
                [
                    [1, 3, 4],
                    [0, 2, 5],
                    [1, 3, 6],
                    [0, 2, 7],
                    [0, 5, 7],
                    [1, 4, 6],
                    [2, 5, 7],
                    [3, 4, 6],
                ]
            ),
            Superclass.M_PARTICLE: np.array([[0]]),
            Superclass.M_INODE: np.array([[0]]),
        }
        if self.value not in node_connections:
            raise NotImplementedError(
                f"This function does not support the superclass {self}. "
                "Please reach out to the mili-python developers."
            )
        return node_connections[self]


class ReturnCode(Enum):
    """Return code enum for _MiliInternal."""

    OK = 0
    ERROR = 1
    CRITICAL = 2

    def str_repr(self) -> str:
        """Get string representation of ReturnCode."""
        return ["Success", "Error", "Critical"][self.value]


ReturnCodeTuple: TypeAlias = Tuple[ReturnCode, str]


class Metadata(TypedDict):
    code_name: str
    username: str
    job_id: str
    nprocs: int
    date: str
    host_name: str
    library_version: str
