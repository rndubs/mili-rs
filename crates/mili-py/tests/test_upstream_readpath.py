"""M4-followup Phase F/G — upstream read-path suite via import redirect.

Aliases ``mili`` (and the submodules the read-path modules import)
onto the milox compatibility surface, then loads the *actual* upstream
test module from ``reference/mili-python/tests/`` and runs its
``unittest`` cases. Skip-not-fail when the submodule corpus is absent
(mirrors conftest.py / the Rust parity tests).

Phase F redirected ``test_reader`` (engine identity + open_database
kwargs). Phase G adds ``test_miliinternal`` (the primal-only reshape
surface) and the **read half** of ``test_milidatabase`` (the
``MiliDatabase`` wrapper: return-code raising + mdg-enum coercion over
the same Rust core). The geometry/derived/projection/adjacency and
write modules stay Phase G-tail / Phase H / Phase 3 — their cases are
honestly ``xfail``ed with the phase that lands them. See
planning/mili-py/m4.md decision 19.
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
    "mili.geometric_mesh_info": milox.geometric_mesh_info,
}

# Tests in redirected modules that exercise still-unported surface.
# Honestly xfailed with the phase that lands them — never silently
# passed, never deleted (planning/mili-py/m4.md decision 19). Strict:
# if one starts passing, the harness fails so it gets promoted.
# Keyed by ``module::Class::method``.
_XFAIL = {
    "test_miliinternal::TestMiliInternal::test_geometry_property": "Phase H: GeometricMeshInfo",
    "test_miliinternal::TestMiliInternal::test_supported_variables": "Phase H: derived engine",
    "test_miliinternal::TestMiliInternal::test_derived_variables_of_class": "Phase H: derived engine",
    "test_miliinternal::TestMiliInternal::test_classes_of_derived_variable": "Phase H: derived engine",
}

# test_milidatabase: milox collapses upstream's per-proc fan-out in the
# Rust DatabaseSet, so the parallel handler classes' merge_results=False
# per-proc-unmerged shapes legitimately differ — that is parallel /
# Phase-H scope, not the Phase-G read half. Whole classes xfail.
_MDB_PARALLEL_CLASSES = (
    "TestReturnCodes",
    "LoopWrapperParallelTests",
    "ServerWrapperParallelTests",
    "LoopWrapperContextManagerParallelTests",
    "ServerWrapperContextManagerParallelTests",
)
# Serial read-half methods that need a still-unported core engine
# (geometry / derived / projection / query result-modifiers — none of
# which the Phase-G primal surface provides). Honest Phase-H xfail.
_MDB_PHASE_H_METHODS = {
    "test_derived_variables_of_class": "Phase H: derived engine",
    "test_query_project_to_nodes": "Phase H: projection engine",
    "test_cummin": "Phase H: query result modifiers (reductions)",
    "test_cummax": "Phase H: query result modifiers (reductions)",
    "test_query_min": "Phase H: query result modifiers (reductions)",
    "test_query_min_dataframe": "Phase H: query result modifiers (reductions)",
    "test_query_max": "Phase H: query result modifiers (reductions)",
    "test_query_max_dataframe": "Phase H: query result modifiers (reductions)",
    "test_query_average": "Phase H: query result modifiers (reductions)",
    "test_query_average_dataframe": "Phase H: query result modifiers (reductions)",
    "test_query_median": "Phase H: query result modifiers (reductions)",
    "test_query_median_dataframe": "Phase H: query result modifiers (reductions)",
    "test_query_stddev": "Phase H: query result modifiers (reductions)",
    "test_query_stddev_dataframe": "Phase H: query result modifiers (reductions)",
}


def _xfail_reason(mod: str, cls: str, meth: str) -> str | None:
    key = f"{mod}::{cls}::{meth}"
    if key in _XFAIL:
        return _XFAIL[key]
    if mod == "test_milidatabase":
        if cls in _MDB_PARALLEL_CLASSES:
            return "parallel handler scope (Rust DatabaseSet collapses the per-proc fan-out; Phase H)"
        if meth in _MDB_PHASE_H_METHODS:
            return _MDB_PHASE_H_METHODS[meth]
    return None


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


def _case_key(test) -> tuple[str, str]:
    parts = test.id().split(".")
    return parts[-2], parts[-1]


_REDIRECTED = ["test_reader", "test_miliinternal", "test_milidatabase"]


def _ids():
    if not (UPSTREAM_TESTS / "test_reader.py").is_file():
        return []
    ids = []
    for m in _REDIRECTED:
        mod = _load_upstream(m)
        for t in _collect(mod):
            cls, meth = _case_key(t)
            ids.append(f"{m}::{cls}::{meth}")
    return ids


_DATA = UPSTREAM_TESTS / "data" / "serial" / "sstate"


@pytest.mark.skipif(
    not _DATA.is_dir(),
    reason="reference/mili-python submodule data absent",
)
@pytest.mark.parametrize("case_id", _ids())
def test_upstream_redirected(case_id):
    mod_name, cls, method = case_id.split("::", 2)
    module = _load_upstream(mod_name)
    case = next(
        t for t in _collect(module) if _case_key(t) == (cls, method)
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
    failed = bool(result.failures or result.errors)
    xfail_reason = _xfail_reason(mod_name, cls, method)
    if xfail_reason is not None:
        if failed:
            pytest.xfail(xfail_reason)
        pytest.fail(
            f"{case_id} now passes — promote it out of the xfail set "
            f"(was: {xfail_reason})",
            pytrace=False,
        )
    if failed:
        msgs = [tb for _, tb in (result.failures + result.errors)]
        pytest.fail("\n".join(msgs), pytrace=False)
