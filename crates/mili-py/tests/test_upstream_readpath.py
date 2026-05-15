"""M4-followup Phase F — upstream read-path suite via import redirect.

Aliases ``mili`` (and the submodules the read-path modules import)
onto the milox compatibility surface, then loads the *actual* upstream
test module from ``reference/mili-python/tests/`` and runs its
``unittest`` cases. Skip-not-fail when the submodule corpus is absent
(mirrors conftest.py / the Rust parity tests).

Phase F redirects only the modules the existing 13-method Rust surface
already satisfies (``test_reader`` — engine identity + open_database
kwargs). The data-bearing read-path modules
(``test_milidatabase``/``test_miliinternal``/``test_derived``/…) come
online in Phases G/H as the core surface lands; the write modules stay
Phase 3. See planning/mili-py/m4.md decision 19.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

import pytest

import milox

REPO_ROOT = Path(__file__).resolve().parents[3]
UPSTREAM_TESTS = REPO_ROOT / "reference" / "mili-python" / "tests"

# Submodule name -> milox module providing the compatible surface.
_REDIRECT = {
    "mili": milox,
    "mili.reader": milox.reader,
    "mili.milidatabase": milox.milidatabase,
    "mili.miliinternal": milox.miliinternal,
    "mili.parallel": milox.parallel,
    "mili.afileIO": milox.afileIO,
    "mili.datatypes": milox.datatypes,
    "mili.mdg_defines": milox.mdg_defines,
}


def _load_upstream(mod_name: str):
    """Import ``reference/.../<mod_name>.py`` with ``mili`` aliased to
    milox, under a non-clashing module name so its ``__file__`` (and
    thus its ``data/`` lookup) stays in the submodule."""
    src = UPSTREAM_TESTS / f"{mod_name}.py"
    saved = {k: sys.modules.get(k) for k in _REDIRECT}
    sys.modules.update(_REDIRECT)
    try:
        spec = importlib.util.spec_from_file_location(
            f"_upstream_{mod_name}", src
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        for k, v in saved.items():
            if v is None:
                sys.modules.pop(k, None)
            else:
                sys.modules[k] = v


def _collect(module):
    loader = unittest.TestLoader()
    suite = loader.loadTestsFromModule(module)
    cases = []
    for grp in suite:
        for test in grp:
            cases.append(test)
    return cases


_REDIRECTED = ["test_reader"]


def _ids():
    if not (UPSTREAM_TESTS / "test_reader.py").is_file():
        return []
    ids = []
    for m in _REDIRECTED:
        mod = _load_upstream(m)
        for t in _collect(mod):
            ids.append(f"{m}::{t.id().split('.')[-1]}")
    return ids


_DATA = UPSTREAM_TESTS / "data" / "serial" / "sstate"


@pytest.mark.skipif(
    not _DATA.is_dir(),
    reason="reference/mili-python submodule data absent",
)
@pytest.mark.parametrize("case_id", _ids())
def test_upstream_redirected(case_id):
    mod_name, method = case_id.split("::", 1)
    module = _load_upstream(mod_name)
    case = next(
        t for t in _collect(module) if t.id().split(".")[-1] == method
    )
    result = unittest.TestResult()
    sys.modules.update(_REDIRECT)
    try:
        case(result)
    finally:
        sys.modules.pop("mili", None)
        for k in _REDIRECT:
            sys.modules.pop(k, None)
    if result.skipped:
        pytest.skip(result.skipped[0][1])
    if result.failures or result.errors:
        msgs = [tb for _, tb in (result.failures + result.errors)]
        pytest.fail("\n".join(msgs), pytrace=False)
