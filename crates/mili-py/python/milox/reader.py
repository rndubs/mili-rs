"""milox ``reader.open_database`` — upstream-compatible entry point.

Mirrors ``mili.reader.open_database``'s signature and handler
selection exactly. The MPI fan-out/merge lives in the Rust
``DatabaseSet``; the ``_MiliInternal`` / ``LoopWrapper`` /
``ServerWrapper`` choice here is preserved purely so callers and the
upstream test suite see the same engine identities.
"""

from __future__ import annotations

import os
from typing import Any, List, Union

from ._native import PyMiliDatabase
from .afileIO import MiliFileNotFoundError, afiles_by_base
from .milidatabase import MiliDatabase
from .miliinternal import _MiliInternal
from .parallel import LoopWrapper, ServerWrapper
from .reductions import combine

__all__ = ["open_database", "combine"]


def open_database(
    base: Union[str, "os.PathLike[str]"],
    procs: List[int] = [],
    suppress_parallel: bool = False,
    experimental: bool = False,
    merge_results: bool = True,
    **kwargs: Any,
) -> MiliDatabase:
    """Open a mili database for querying.

    Same contract as ``mili.reader.open_database``: a single A-file (or
    a ``procs`` subset of size one) yields an ``_MiliInternal``-backed
    handle; multiple fragments yield ``LoopWrapper`` (when
    ``suppress_parallel``) or ``ServerWrapper``. The underlying engine
    is always the Rust ``PyMiliDatabase`` (single vs. set).
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
    afiles = afiles_by_base(dir_name, file_base, procs)

    if len(afiles) == 1:
        engine_db = PyMiliDatabase.open_single(
            os.path.join(dir_name, afiles[0])
        )
        mili_engine: Any = _MiliInternal(engine_db)
    else:
        # Decision 21: the wrapper holds a real per-proc list of
        # _MiliInternal, each opening one fragment's A-file (drop the
        # trailing "A" to get each proc's A/T/S base) — upstream's
        # exact contract. Each per-fragment open is open_single
        # (serial-gate bit-exact); the merge moves to
        # MiliDatabase's per-method reduce_function table.
        proc_bases = [afile[:-1] for afile in afiles]
        Wrapper = LoopWrapper if suppress_parallel else ServerWrapper
        mili_engine = Wrapper(
            _MiliInternal, dir_name, proc_bases, merge_results, **kwargs
        )

    return MiliDatabase(mili_engine, merge_results)
