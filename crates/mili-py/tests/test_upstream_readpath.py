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
    "mili.adjacency": milox.adjacency,
    "mili.reductions": milox.reductions,
    "mili.utils": milox.utils,
    "mili.projection": milox.projection,
}

# Tests in redirected modules that exercise still-unported surface.
# Honestly xfailed with the phase that lands them — never silently
# passed, never deleted (planning/mili-py/m4.md decision 19). Strict:
# if one starts passing, the harness fails so it gets promoted.
# Keyed by ``module::Class::method``.
# The derived-variable *listing* surface landed (Phase H listing
# sub-slice): test_miliinternal's three listing tests now pass, so they
# are no longer xfailed. Nothing else in this dict yet.
_XFAIL: dict[str, str] = {}

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
# Serial read-half methods that need a still-unported core engine.
# The projection sub-slice landed (milox.projection verbatim port +
# MiliDatabase._project_result), so test_query_project_to_nodes is no
# longer xfailed — nothing else in the Phase-G read half is unported.
_MDB_PHASE_H_METHODS: dict[str, str] = {}
# Parallel-class methods that legitimately *pass* now that the
# ResultModifier reductions are ported: the AVERAGE dataframe reduces to
# one partition-independent value per state (no per-proc label axis), so
# the Rust-DatabaseSet-merged result is bit-identical to upstream's
# per-proc-combined result. The other reduction methods keep a
# label/argmin axis that differs per partition → still honest parallel
# xfail. Excluded from the whole-class xfail so the strict harness
# asserts the pass.
_MDB_PARALLEL_PASSING = {
    "test_query_average_dataframe",
}


# test_adjacency: the serial GeometricMeshInfo / AdjacencyMapping
# classes are the Phase-H adjacency sub-slice and must pass. The
# LoopWrapper* / ServerWrapper* (incl. *MergeResults*) classes drive
# the per-proc-unmerged parallel handlers — milox collapses that
# fan-out in the Rust DatabaseSet, so they legitimately differ
# (parallel scope, like _MDB_PARALLEL_CLASSES). Whole classes xfail.
_ADJ_PARALLEL_CLASSES = (
    "LoopWrapperGeometricMeshInfoTests",
    "LoopWrapperAdjacencyMappingTests",
    "ServerWrapperGeometricMeshInfoTests",
    "ServerWrapperAdjacencyMappingTests",
    "LoopWrapperGeometricMeshInfoMergeResultsTests",
    "LoopWrapperAdjacencyMappingMergeResultsTests",
    "ServerWrapperGeometricMeshInfoMergeResultsTests",
    "ServerWrapperAdjacencyMappingMergeResultsTests",
)


# test_reductions: TestSerialReductions opens the *serial* d3samp6
# twice (merge_results False/True) — both single-fragment _MiliInternal,
# so the serial-vs-"parallel" comparison is the already-ported primal /
# reshape / geometry surface and must pass. TestCombineFunction /
# TestMergeDataFrames / TestServerWrapperReductions /
# TestLoopWrapperReductions drive the *parallel* d3samp6 through the
# per-proc-unmerged handlers (merge_results=False expects upstream's
# per-proc list; milox collapses that fan-out in the Rust DatabaseSet),
# so they legitimately differ — parallel scope, whole-class xfail like
# _MDB_PARALLEL_CLASSES / _ADJ_PARALLEL_CLASSES.
# TestCombineFunction drives the parallel d3samp6 with
# merge_results=False and its helper iterates the upstream per-proc
# *list*; milox returns the Rust-DatabaseSet-merged dict, so every
# method legitimately differs (whole-class parallel scope, all fail —
# no incidental pass).
_REDUCTIONS_COMBINE_CLASS = "TestCombineFunction"
# TestServerWrapperReductions / TestLoopWrapperReductions compare the
# *serial* d3samp6 against the *parallel* d3samp6 (a different, per-proc
# database) and call the ServerWrapper/LoopWrapper forwarding path
# (per-proc-unmerged list shapes / raw PyMiliDatabase signatures); milox
# collapses that fan-out in the Rust core, so these legitimately differ.
# Parallel-handler scope, exactly like _MDB_PARALLEL_CLASSES — the
# identical 22-method set fails for both wrapper classes; the remaining
# methods incidentally agree (loose type/sorted checks) and must pass,
# so the strict harness needs the precise per-method set, not a
# whole-class xfail.
_REDUCTIONS_WRAPPER_METHODS = {
    "test_all_labels_of_material",
    "test_append_state",
    "test_class_labels_of_material",
    "test_classes_of_derived_variable",
    "test_classes_of_state_variable",
    "test_components_of_vector_svar",
    "test_connectivity",
    "test_containing_state_variables_of_class",
    "test_copy_non_state_data",
    "test_derived_variables_of_class",
    "test_int_points_of_state_variable",
    "test_labels",
    "test_materials_of_class_name",
    "test_nodes",
    "test_nodes_of_elems",
    "test_nodes_of_material",
    "test_parts_of_class_name",
    "test_queriable_svars",
    "test_state_variable_titles",
    "test_state_variables_of_class",
    "test_supported_derived_variables",
    "test_times",
}
_REDUCTIONS_WRAPPER_CLASSES = (
    "TestServerWrapperReductions",
    "TestLoopWrapperReductions",
)
# TestMergeDataFrames queries the parallel d3samp6 brick/beam with
# per-proc-local labels (e.g. brick label 20 lives on one proc only);
# the merged Rust result legitimately differs from the per-proc-unmerged
# expectation. node-class methods use global labels and pass.
_REDUCTIONS_MERGEDF_CLASS = "TestMergeDataFrames"
_REDUCTIONS_MERGEDF_METHODS = {
    "test_multiple_scalars",
    "test_single_scalar",
    "test_vector",
    "test_vector_array",
}
# Serial methods that still route through a genuinely-unported engine.
# The derived-variable *listing* surface landed (Phase H listing
# sub-slice) so those promoted to green; only the Phase-3 write path
# remains here. Honest strict-xfail, never silently passed.
_REDUCTIONS_PHASE_H_METHODS: dict[str, str] = {}
_REDUCTIONS_WRITE_METHODS = {
    "test_append_state": "Phase 3 write path",
    "test_copy_non_state_data": "Phase 3 write path",
}


# test_derived: the derived-variable *listing* surface landed (Phase H
# listing sub-slice) — the three SerialDerivedExpressions listing tests
# must pass. Every other test_derived method exercises the derived
# *value* engine (stress/strain invariants, velocities, accelerations,
# …) which is the next Phase-H sub-slice (parity-sensitive value math;
# Rust core per decision 19 — node displacement already landed there).
# Honest strict-xfail. ParallelDerivedExpressions additionally drives
# the per-proc-unmerged parallel d3samp6 handlers (parallel scope, like
# _MDB_PARALLEL_CLASSES). No listing tests on the parallel class.
_DERIVED_LISTING_METHODS = {
    "test_supported_variables",
    "test_derived_variables_of_class",
    "test_classes_of_derived_variable",
}
# Serial nodal-kinematics *value* tests in the Rust core
# (mili_rs::derived), all routing through MiliDatabase.query -> Rust
# (not milox.derived): disp_x/disp_y/disp_z (prior reductions
# sub-slice), the displacement-magnitude + reference_state extension
# (disp_mag/disp_rad_mag_xy/non-zero reference_state), and the
# velocity/acceleration finite-difference family
# (vel_*/acc_* — f32 single-prec d3samp6 for acc, f64 double-prec
# solids014_dblplt for vel; f32 per-state-time arithmetic mirrors
# numpy NEP50 promotion). The scalar (non-eigenvalue) stress
# invariants (pressure / eff_stress / triaxiality / norm_press) are
# pure element-wise arithmetic over the 6 stress component primals
# on the requested element class (f32/f64-generic, numpy NEP50 weak-
# scalar promotion). The eigenvalue-based stress invariants
# (prin_stress1-3 / prin_dev_stress1-3 / max_shear_stress) build the
# symmetric stress (or deviatoric) 3x3 from the 6 component primals
# and read a symmetric-3x3 Jacobi eigensolver in the core (computed
# in f64, cast to the primal dtype — bit-identical to numpy's f32
# eigvalsh at every literal-checked point). vol_strain is the trivial
# strain trace; prin_strain* / prin_dev_strain* reuse that same
# eigensolver on the 6 strain components. nodtangmag /
# shear_magnitude are the sqrt-of-sum-of-component-squares pattern
# (like disp_mag, generic f32/f64, no connectivity). The *_alt griz
# closed-form trig variants have no value-test (listing-only) and the
# connectivity-coupled geometry derived
# (centroid/element_volume/area/force/surfstrain) + projection are
# later sub-slices (an architectural decision point — they thread
# connectivity / cross-derived deps / projection into the derived
# query routing).
_DERIVED_SERIAL_PASSING = {
    "test_disp_x",
    "test_disp_y",
    "test_disp_z",
    "test_disp_mag",
    "test_disp_mag_ref",
    "test_disp_rad_mag_xy",
    "test_disp_y_reference_state",
    "test_vel_x",
    "test_vel_y",
    "test_vel_z",
    "test_acc_x",
    "test_acc_y",
    "test_acc_z",
    "test_pressure",
    "test_eff_stress",
    "test_triaxiality",
    "test_normalized_pressure",
    "test_query_multiple_variables",
    "test_prin_stress1",
    "test_prin_stress2",
    "test_prin_stress3",
    "test_prin_dev_stress1",
    "test_prin_dev_stress2",
    "test_prin_dev_stress3",
    "test_max_shear_stress",
    "test_vol_strain",
    "test_prin_strain1",
    "test_prin_strain2",
    "test_prin_strain3",
    "test_prin_dev_strain1",
    "test_prin_dev_strain2",
    "test_prin_dev_strain3",
    "test_shear_magnitude",
    "test_eps_rate",
    "test_hex_centroid",
    "test_beam_centroid",
    "test_shell_centroid",
    "test_node_centroid",
    "test_hex_element_volume",
    "test_tet_element_volume",
    "test_quad_area",
    "test_tet_relative_volume",
    "test_dyna_normal_force",
    "test_force_x",
    "test_force_y",
    "test_force_z",
    "test_mat_cog_disp",
    # The dbl_nodtang (diablo) core primal-query label-resolution bug
    # is fixed (subrecord `id_blocks` enumerate 1-based class mo ids,
    # not user labels — they coincide only for contiguous `1..=qty`
    # label classes; `cbs1_particle` is `[5,10,..,125]` over mo ids
    # `1..=25`). `nodtangmag` is a pure sqrt-of-squares magnitude with
    # no internal geometry query, so it now passes outright.
    "test_nodetangmag",
    # The geometry-derived path is now generic over the primal `nodpos`
    # dtype (`mili_rs::geometry` GeomF kernels + `compute_contact_force`
    # f64): dbl_nodtang is double-precision, so `relative_volume`
    # (f64-internal jacobian, f32 result) and `normal_force` (f64
    # nodpres × the f64 M_QUAD area) are now bit-exact vs the upstream
    # oracle. The f32 path is unchanged (same ops / exact divisors).
    "test_hex_relative_volume",
    "test_diablo_normal_force",
    # The per-face Hex `surfstrain{x,y,z,xy,yz,zx}` derived + the
    # `face=` query kwarg landed (core `surface_strain_query`). It is
    # bit-exact vs the upstream oracle on dbl_nodtang for every
    # component/face the test exercises (faces 1 & 3 asserted, all 6
    # checked), mirroring numpy's mixed precision exactly: the
    # `ux/uy/uz` = `nodpos` positions are f64 (double-precision
    # corpus) and `disp = pos - ref` stays f64, while every
    # `np.empty(..., dtype=np.float32)` intermediate is f32 with the
    # same op order, so the f64→f32 truncation points land
    # identically. Missing / out-of-range `face` raises
    # MiliPythonError (mirrors upstream's ValueError surface).
    "test_surfstrain",
}
# No remaining dbl_nodtang derived-value blockers; surfstrain + the
# `face=` kwarg landed (bit-exact vs the oracle), so the entire
# serial derived value engine is green. The only remaining Phase-H
# derived item is the `project_to_nodes` projection layer — a
# distinct architectural slice (a Python `mili.projection` module +
# the parallel-handler `grizinterface`), tracked by its own
# honest-xfail set / module gates below, not here.
_DERIVED_DBL_NODTANG_BLOCKED: set[str] = set()
_DERIVED_DBL_NODTANG_REASON = (
    "dbl_nodtang derived-value path is complete (label-resolution + "
    "geometry-derived f64 + surfstrain all landed); no remaining "
    "blocker."
)


def _xfail_reason(mod: str, cls: str, meth: str) -> str | None:
    key = f"{mod}::{cls}::{meth}"
    if key in _XFAIL:
        return _XFAIL[key]
    if mod == "test_milidatabase":
        if cls in _MDB_PARALLEL_CLASSES and meth not in _MDB_PARALLEL_PASSING:
            return "parallel handler scope (Rust DatabaseSet collapses the per-proc fan-out; Phase H)"
        if meth in _MDB_PHASE_H_METHODS:
            return _MDB_PHASE_H_METHODS[meth]
    if mod == "test_derived":
        if cls == "SerialDerivedExpressions" and (
            meth in _DERIVED_LISTING_METHODS
            or meth in _DERIVED_SERIAL_PASSING
        ):
            return None
        if cls == "ParallelDerivedExpressions":
            return (
                "parallel handler scope + derived value engine (Rust "
                "DatabaseSet collapses the per-proc fan-out; Phase H "
                "derived value sub-slice)"
            )
        if cls == "SerialDerivedExpressions":
            if meth in _DERIVED_DBL_NODTANG_BLOCKED:
                return _DERIVED_DBL_NODTANG_REASON
        return "Phase H: derived value engine (next sub-slice)"
    if mod == "test_adjacency" and cls in _ADJ_PARALLEL_CLASSES:
        return "parallel handler scope (Rust DatabaseSet collapses the per-proc fan-out; Phase H)"
    if mod == "test_reductions":
        if cls == _REDUCTIONS_COMBINE_CLASS:
            return "parallel handler scope (Rust DatabaseSet collapses the per-proc fan-out; Phase H)"
        if (
            cls in _REDUCTIONS_WRAPPER_CLASSES
            and meth in _REDUCTIONS_WRAPPER_METHODS
        ):
            return "parallel handler scope (serial vs per-proc parallel d3samp6; Rust DatabaseSet collapses the fan-out; Phase H)"
        if (
            cls == _REDUCTIONS_MERGEDF_CLASS
            and meth in _REDUCTIONS_MERGEDF_METHODS
        ):
            return "parallel handler scope (per-proc-local labels in parallel d3samp6; Phase H)"
        if cls == "TestSerialReductions":
            if meth in _REDUCTIONS_WRITE_METHODS:
                return _REDUCTIONS_WRITE_METHODS[meth]
            if meth in _REDUCTIONS_PHASE_H_METHODS:
                return _REDUCTIONS_PHASE_H_METHODS[meth]
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


_REDIRECTED = [
    "test_reader",
    "test_miliinternal",
    "test_milidatabase",
    "test_adjacency",
    "test_reductions",
    "test_derived",
    "test_projection",
]


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
