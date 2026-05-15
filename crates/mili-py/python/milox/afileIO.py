"""milox ``afileIO`` compatibility shim.

A-file *parsing* lives in the Rust core; only the pure discovery
helper (`afiles_by_base`) and the exception identities the read-path
suite imports are needed at the Python layer. `MiliFileNotFoundError`
re-exports the **native** class so `raise`/`except` identity is shared
with the extension (`reader.open_database` raises the native one).
"""

from __future__ import annotations

import os
import re
from typing import List, Optional

from ._native import MiliAParseError, MiliFileNotFoundError

__all__ = ["MiliFileNotFoundError", "MiliAParseError", "afiles_by_base"]


def afiles_by_base(
    dir_name: str, base_filename: str, proc_whitelist: List[int] = []
) -> List[str]:
    """Verbatim port of ``mili.afileIO.afiles_by_base``.

    Discover ``<base>(\\d*)A`` files under ``dir_name``, optionally
    restricted to a process whitelist. Pure stdlib — no parity surface.
    """
    file_re = re.compile(re.escape(base_filename) + r"(\d*)A$")
    files = list(filter(file_re.match, os.listdir(dir_name)))
    files.sort()

    def proc_from_file(fn: str) -> Optional[int]:
        proc_match = file_re.match(fn)
        proc = proc_match.group(1) if proc_match is not None else ""
        return int(proc) if proc != "" else None

    procs_we_have = list(proc_from_file(file) for file in files)
    procs_we_have = [proc for proc in procs_we_have if proc is not None]
    proc_whitelist = [int(proc) for proc in proc_whitelist]
    proc_whitelist = procs_we_have if len(proc_whitelist) == 0 else proc_whitelist
    to_drop = list(set(procs_we_have) - set(proc_whitelist))

    files = [file for file in files if proc_from_file(file) not in to_drop]

    if len(files) == 0:
        raise MiliFileNotFoundError(
            f"No A-files for procs "
            f"'{', '.join([str(proc) for proc in proc_whitelist])}' "
            f"with base name '{base_filename}' discovered in {dir_name}!"
        )
    return files
