"""milox — Rust-backed, API-compatible mili reader.

Surface: M1–M4 read path + Slice B, plus the M4-followup **Phase F**
``mili``-compatible package skeleton (`reader`, `milidatabase`,
`miliinternal`, `parallel`, `afileIO`, `datatypes`, `mdg_defines`) so
the upstream read-path test suite can run under an import redirect.
See ``planning/mili-py/m4.md`` decision 19 for the phased plan.
"""

from __future__ import annotations

import os
from typing import Any, List, Union

from ._native import (
    MiliAParseError,
    MiliError,
    MiliFileNotFoundError,
    MiliPythonError,
    PyMiliDatabase,
)
from . import (
    adjacency,
    afileIO,
    datatypes,
    geometric_mesh_info,
    mdg_defines,
    milidatabase,
    miliinternal,
    parallel,
    reader,
)
from .milidatabase import MiliDatabase
from .reader import open_database as _open_database

__all__ = [
    "open_database",
    "PyMiliDatabase",
    "MiliDatabase",
    "MiliError",
    "MiliFileNotFoundError",
    "MiliAParseError",
    "MiliPythonError",
    "reader",
    "milidatabase",
    "miliinternal",
    "parallel",
    "afileIO",
    "datatypes",
    "mdg_defines",
    "geometric_mesh_info",
    "adjacency",
]


def open_database(
    base: Union[str, "os.PathLike[str]"],
    procs: List[int] = [],
    suppress_parallel: bool = False,
    experimental: bool = False,
    merge_results: bool = True,
    **kwargs: Any,
) -> MiliDatabase:
    """Top-level convenience alias for :func:`milox.reader.open_database`.

    Returns a :class:`MiliDatabase` whose read accessors forward to the
    Rust core — so existing ``milox.open_database(path).query(...)``
    call sites keep working unchanged.
    """
    return _open_database(
        base,
        procs=procs,
        suppress_parallel=suppress_parallel,
        experimental=experimental,
        merge_results=merge_results,
        **kwargs,
    )
