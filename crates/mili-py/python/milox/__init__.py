"""milox — Rust-backed, API-compatible mili reader.

M1 surface: ``open_database()`` plus read-only metadata accessors. The
filename-root parsing lives here (it predates the FFI boundary); the
Rust core takes a fully-resolved path.
"""

from __future__ import annotations

import os
import re
from typing import List, Union

from ._native import (
    MiliAParseError,
    MiliError,
    MiliFileNotFoundError,
    MiliPythonError,
    PyMiliDatabase,
)

__all__ = [
    "open_database",
    "PyMiliDatabase",
    "MiliError",
    "MiliFileNotFoundError",
    "MiliAParseError",
    "MiliPythonError",
]


def _afiles_by_base(dir_name: str, base: str) -> List[str]:
    """Port of ``mili.afileIO.afiles_by_base`` (the discovery half).

    Matches ``re.escape(base) + r"(\\d*)A$"`` against the directory and
    returns the sorted A-file names.
    """
    file_re = re.compile(re.escape(base) + r"(\d*)A$")
    files = [f for f in os.listdir(dir_name) if file_re.match(f)]
    files.sort()
    return files


def open_database(base: Union[str, os.PathLike]) -> PyMiliDatabase:
    """Open a mili database for read-only metadata access.

    Mirrors ``mili.reader.open_database``: a single A-file yields a
    ``Database``-backed object; multiple fragments yield a
    ``DatabaseSet``-backed one (the fan-out/merge lives in Rust).
    """
    base = os.fspath(base)
    dir_name = os.path.dirname(base)
    if dir_name == "":
        dir_name = os.getcwd()
    if not os.path.isdir(dir_name):
        raise MiliFileNotFoundError(
            f"Cannot locate mili file directory {dir_name}."
        )

    file_base = os.path.basename(base)
    afiles = _afiles_by_base(dir_name, file_base)
    if len(afiles) == 0:
        raise MiliFileNotFoundError(
            f"No A files with the basename '{file_base}' were found in "
            f"the directory '{dir_name}'"
        )

    if len(afiles) == 1:
        return PyMiliDatabase.open_single(os.path.join(dir_name, afiles[0]))
    return PyMiliDatabase.open_set(base)
