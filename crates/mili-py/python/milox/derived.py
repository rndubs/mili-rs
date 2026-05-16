"""milox derived-variable engine — listing sub-slice (Phase H).

The derived-variable *listing* surface — ``supported_variables``,
``derived_variable_titles``, ``derived_variables_of_class``,
``classes_of_derived_variable`` — is pure metadata: a static
expression table plus set-membership over the already-ported core
accessors (``classes_of_state_variable`` / ``mesh_object_classes`` /
``queriable_svars`` / ``class_names``). It is **not** parity-sensitive
value/topology computation, so per the architecture invariant and the
decision-18/19 precedent (reductions, the GeometricMeshInfo adapter)
it is ported **verbatim** from
``reference/mili-python/src/mili/derived.py`` into milox.

The ``__derived_expressions`` table is copied verbatim (titles /
primals / primals_class / only_sclasses / alternate_primals /
supports_batching). The value ``compute_function``s are *not* this
sub-slice — they are parity-sensitive value math that belongs in the
Rust core (decision 19; node displacement already landed in
``mili_rs::derived``). They are wired to an explicit typed-error stub
so ``DerivedExpressions.query`` / ``find_batchable_queries`` raise
``MiliPythonError`` naming the next sub-slice — never a silent wrong
answer. See planning/mili-py/m4.md decision 19.
"""

from __future__ import annotations

from itertools import groupby
from typing import TYPE_CHECKING, Any, Callable, Dict, List, Optional

from typing_extensions import NotRequired, TypedDict

from ._native import MiliPythonError
from .datatypes import Superclass
from .mdg_defines import (
    ContactSegmentStateVariables,
    DerivedVariables,
    EntityType,
    MaterialStateVariables,
    NodalStateVariables,
    ShellStateVariables,
    StressStrainStateVariables,
)

if TYPE_CHECKING:
    from .miliinternal import _MiliInternal

__all__ = ["DerivedExpressions", "DerivedSpec"]


class DerivedSpec(TypedDict):
    title: str
    primals: List[str]
    alternate_primals: NotRequired[List[str]]
    primals_class: List[Optional[str]]
    supports_batching: bool
    compute_function: Callable[..., Any]
    only_sclasses: NotRequired[List[Superclass]]


class DerivedExpressions:
    """Derived-variable listing over the milox ``_MiliInternal`` engine.

    Verbatim port of upstream ``mili.derived.DerivedExpressions`` for
    the listing surface. The value-compute methods are the next Phase-H
    sub-slice — wired to ``__value_not_ported`` (explicit typed error).
    """

    def __init__(self, db: "_MiliInternal") -> None:
        self.db = db

        nv = self.__value_not_ported
        self.__derived_expressions: Dict[str, DerivedSpec] = {
            DerivedVariables.X_DISPLACEMENT.value: DerivedSpec(
                title="X Displacement",
                primals=[NodalStateVariables.X_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.Y_DISPLACEMENT.value: DerivedSpec(
                title="Y Displacement",
                primals=[NodalStateVariables.Y_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.Z_DISPLACEMENT.value: DerivedSpec(
                title="Z Displacement",
                primals=[NodalStateVariables.Z_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.DISPLACEMENT_MAGNITUDE.value: DerivedSpec(
                title="Displacement Magnitude",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.RADIAL_DISPLACEMENT_MAGNITUDE_XY.value: DerivedSpec(
                title="Radial Displacement Magnitude XY",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                ],
                primals_class=[None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.X_VELOCITY.value: DerivedSpec(
                title="X Velocity",
                primals=[NodalStateVariables.X_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.Y_VELOCITY.value: DerivedSpec(
                title="Y Velocity",
                primals=[NodalStateVariables.Y_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.Z_VELOCITY.value: DerivedSpec(
                title="Z Velocity",
                primals=[NodalStateVariables.Z_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.X_ACCELERATION.value: DerivedSpec(
                title="X Acceleration",
                primals=[NodalStateVariables.X_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.Y_ACCELERATION.value: DerivedSpec(
                title="Y Acceleration",
                primals=[NodalStateVariables.Y_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.Z_ACCELERATION.value: DerivedSpec(
                title="Z Acceleration",
                primals=[NodalStateVariables.Z_POSITION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.VOLUMETRIC_STRAIN.value: DerivedSpec(
                title="Volumetric Strain",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                ],
                primals_class=[None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRAIN_1.value: DerivedSpec(
                title="Principal Strain 1",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRAIN_2.value: DerivedSpec(
                title="Principal Strain 2",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRAIN_3.value: DerivedSpec(
                title="Principal Strain 3",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRAIN_1.value: DerivedSpec(
                title="Principal Deviatoric Strain 1",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRAIN_2.value: DerivedSpec(
                title="Principal Deviatoric Strain 2",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRAIN_3.value: DerivedSpec(
                title="Principal Deviatoric Strain 3",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRAIN_1_ALT.value: DerivedSpec(
                title="Principal Strain 1 (alt)",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRAIN_2_ALT.value: DerivedSpec(
                title="Principal Strain 2 (alt)",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRAIN_3_ALT.value: DerivedSpec(
                title="Principal Strain 3 (alt)",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRAIN_1_ALT.value: DerivedSpec(
                title="Principal Deviatoric Strain 1 (alt)",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRAIN_2_ALT.value: DerivedSpec(
                title="Principal Deviatoric Strain 2 (alt)",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRAIN_3_ALT.value: DerivedSpec(
                title="Principal Deviatoric Strain 3 (alt)",
                primals=[
                    StressStrainStateVariables.X_STRAIN.value,
                    StressStrainStateVariables.Y_STRAIN.value,
                    StressStrainStateVariables.Z_STRAIN.value,
                    StressStrainStateVariables.XY_STRAIN.value,
                    StressStrainStateVariables.YZ_STRAIN.value,
                    StressStrainStateVariables.ZX_STRAIN.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRESS_1.value: DerivedSpec(
                title="Principal Stress 1",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRESS_2.value: DerivedSpec(
                title="Principal Stress 2",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_STRESS_3.value: DerivedSpec(
                title="Principal Stress 3",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.EFFECTIVE_STRESS.value: DerivedSpec(
                title="Effective Stress",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRESSURE.value: DerivedSpec(
                title="Pressure",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                ],
                primals_class=[None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRESS_1.value: DerivedSpec(
                title="Principal Deviatoric Stress 1",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRESS_2.value: DerivedSpec(
                title="Principal Deviatoric Stress 2",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.PRINCIPAL_DEV_STRESS_3.value: DerivedSpec(
                title="Principal Deviatoric Stress 3",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=True,
                compute_function=nv,
            ),
            DerivedVariables.MAX_SHEAR_STRESS.value: DerivedSpec(
                title="Maximum Shear Stress",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.TRIAXIALITY.value: DerivedSpec(
                title="Triaxiality",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.NORMALIZED_PRESSURE.value: DerivedSpec(
                title="Normalized Pressure",
                primals=[
                    StressStrainStateVariables.X_STRESS.value,
                    StressStrainStateVariables.Y_STRESS.value,
                    StressStrainStateVariables.Z_STRESS.value,
                    StressStrainStateVariables.XY_STRESS.value,
                    StressStrainStateVariables.YZ_STRESS.value,
                    StressStrainStateVariables.ZX_STRESS.value,
                ],
                primals_class=[None, None, None, None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.EPS_RATE.value: DerivedSpec(
                title="Equiv. Plastic Strain Rate",
                primals=[StressStrainStateVariables.EQUIV_PLASTIC_STRAIN.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.TANGENTIAL_TRACTION_MAGNITUDE.value: DerivedSpec(
                title="Nodal Tangential Traction Magnitude",
                primals=[
                    NodalStateVariables.X_TANGENTIAL_TRACTION.value,
                    NodalStateVariables.Y_TANGENTIAL_TRACTION.value,
                    NodalStateVariables.Z_TANGENTIAL_TRACTION.value,
                ],
                primals_class=[None, None, None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.MAT_COG_DISP_X.value: DerivedSpec(
                title="Material Center of Gravity X Displacement",
                primals=[
                    MaterialStateVariables.CENTER_OF_GRAVITY_X_POSITION.value
                ],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.MAT_COG_DISP_Y.value: DerivedSpec(
                title="Material Center of Gravity Y Displacement",
                primals=[
                    MaterialStateVariables.CENTER_OF_GRAVITY_Y_POSITION.value
                ],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.MAT_COG_DISP_Z.value: DerivedSpec(
                title="Material Center of Gravity Z Displacement",
                primals=[
                    MaterialStateVariables.CENTER_OF_GRAVITY_Z_POSITION.value
                ],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.ELEMENT_VOLUME.value: DerivedSpec(
                title="Element Volume",
                primals=[NodalStateVariables.NODAL_POSITION.value],
                primals_class=[EntityType.NODE.value],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX, Superclass.M_TET],
            ),
            DerivedVariables.AREA.value: DerivedSpec(
                title="Quad Area",
                primals=[NodalStateVariables.NODAL_POSITION.value],
                primals_class=[EntityType.NODE.value],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_QUAD],
            ),
            DerivedVariables.CENTROID.value: DerivedSpec(
                title="Centroid Position",
                primals=[NodalStateVariables.NODAL_POSITION.value],
                primals_class=[EntityType.NODE.value],
                supports_batching=False,
                compute_function=nv,
            ),
            DerivedVariables.SURFACE_STRAIN_X.value: DerivedSpec(
                title="Surface Strain X",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                ],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX],
            ),
            DerivedVariables.SURFACE_STRAIN_Y.value: DerivedSpec(
                title="Surface Strain Y",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                ],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX],
            ),
            DerivedVariables.SURFACE_STRAIN_Z.value: DerivedSpec(
                title="Surface Strain Z",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                ],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX],
            ),
            DerivedVariables.SURFACE_STRAIN_XY.value: DerivedSpec(
                title="Surface Strain XY",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                ],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX],
            ),
            DerivedVariables.SURFACE_STRAIN_YZ.value: DerivedSpec(
                title="Surface Strain YZ",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                ],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX],
            ),
            DerivedVariables.SURFACE_STRAIN_ZX.value: DerivedSpec(
                title="Surface Strain ZX",
                primals=[
                    NodalStateVariables.X_POSITION.value,
                    NodalStateVariables.Y_POSITION.value,
                    NodalStateVariables.Z_POSITION.value,
                ],
                primals_class=[
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                    EntityType.NODE.value,
                ],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX],
            ),
            DerivedVariables.RELATIVE_VOLUME.value: DerivedSpec(
                title="Relative Volume",
                primals=[NodalStateVariables.NODAL_POSITION.value],
                primals_class=[EntityType.NODE.value],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_HEX, Superclass.M_TET],
            ),
            DerivedVariables.NORMAL_FORCE.value: DerivedSpec(
                title="Normal Force",
                primals=[ContactSegmentStateVariables.DYNA_NORMAL_PRESSURE.value],
                alternate_primals=[
                    ContactSegmentStateVariables.DIABLO_NORMAL_PRESSURE.value
                ],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_QUAD],
            ),
            DerivedVariables.FORCE_X.value: DerivedSpec(
                title="X Force",
                primals=[ContactSegmentStateVariables.X_TRACTION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_QUAD],
            ),
            DerivedVariables.FORCE_Y.value: DerivedSpec(
                title="Y Force",
                primals=[ContactSegmentStateVariables.Y_TRACTION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_QUAD],
            ),
            DerivedVariables.FORCE_Z.value: DerivedSpec(
                title="Z Force",
                primals=[ContactSegmentStateVariables.Z_TRACTION.value],
                primals_class=[None],
                supports_batching=False,
                compute_function=nv,
                only_sclasses=[Superclass.M_QUAD],
            ),
            DerivedVariables.SHEAR_MAGNITUDE.value: DerivedSpec(
                title="Shear Magnitude",
                primals=[
                    ShellStateVariables.SHEAR_XX.value,
                    ShellStateVariables.SHEAR_YY.value,
                ],
                primals_class=[None, None],
                supports_batching=False,
                compute_function=nv,
            ),
        }

    def __value_not_ported(self, *args: Any, **kwargs: Any) -> Any:
        """Placeholder for the not-yet-ported value-compute sub-slice.

        The derived *value* engine (stress/strain invariants,
        velocities, accelerations, …) is parity-sensitive math that
        belongs in the Rust core (decision 19; node displacement
        already landed in ``mili_rs::derived``). Until that sub-slice
        lands, raise an explicit typed error rather than return a
        silent wrong answer.
        """
        raise MiliPythonError(
            "derived value computation: not yet ported (mili-py "
            "M4-followup Phase H derived value sub-slice; see "
            "planning/mili-py/m4.md decision 19)"
        )

    def supported_variables(self) -> List[str]:
        """Return a list of derived expressions that are supported.

        NOTE: This does not mean all derived variables can be calculated
              for a given simulation. Only that mili-python can
              caclulate them if all required inputs exist.
        """
        return list(self.__derived_expressions.keys())

    def derived_variable_titles(self) -> Dict[str, str]:
        """Return dictionary containing the title for each derived variable."""
        return {
            var: spec["title"]
            for var, spec in self.__derived_expressions.items()
        }

    def __variable_exists_for_class(
        self, variable: str, class_name: str
    ) -> bool:
        primal_exists = class_name in self.db.classes_of_state_variable(variable)
        try:
            derived_exists = class_name in self.db.classes_of_derived_variable(
                variable
            )
        except Exception:
            derived_exists = False
        self.db.clear_return_code()
        return primal_exists or derived_exists

    def derived_variables_of_class(self, class_name: str) -> List[str]:
        """Return list of derived variables that can be calculated for a given class."""
        derived_list = []
        if class_name in self.db.class_names():
            class_def = self.db.mesh_object_classes()[class_name]
            queriable_state_variables = self.db.queriable_svars()
            for var_name, spec in self.__derived_expressions.items():
                if "only_sclasses" in spec:
                    if class_def.sclass not in spec["only_sclasses"]:
                        continue
                primals_found = []
                for req_primal, req_primal_class in zip(
                    spec["primals"], spec["primals_class"]
                ):
                    # Check that primal exists
                    if (
                        req_primal in queriable_state_variables
                        or req_primal in self.__derived_expressions
                    ):
                        # Check that primal exists for required element class
                        req_primal_class = (
                            class_name
                            if req_primal_class is None
                            else req_primal_class
                        )
                        if self.__variable_exists_for_class(
                            req_primal, req_primal_class
                        ):
                            primals_found.append(True)
                # Check if all primals were found
                if len(primals_found) == len(spec["primals"]) and all(
                    primals_found
                ):
                    derived_list.append(var_name)
                elif "alternate_primals" in spec:
                    primals_found = []
                    for req_primal, req_primal_class in zip(
                        spec["alternate_primals"], spec["primals_class"]
                    ):
                        # Check that primal exists
                        if (
                            req_primal in queriable_state_variables
                            or req_primal in self.__derived_expressions
                        ):
                            # Check that primal exists for required element class
                            req_primal_class = (
                                class_name
                                if req_primal_class is None
                                else req_primal_class
                            )
                            if self.__variable_exists_for_class(
                                req_primal, req_primal_class
                            ):
                                primals_found.append(True)
                    # Check if all primals were found
                    if len(primals_found) == len(spec["primals"]) and all(
                        primals_found
                    ):
                        derived_list.append(var_name)

        return derived_list

    def classes_of_derived_variable(self, var_name: str) -> List[str]:
        """Return list of element classes for which the specified derived variable can be calculated."""
        if var_name not in self.__derived_expressions:
            raise KeyError(f"The derived result '{var_name}' does not exist")
        derived_spec = self.__derived_expressions[var_name]
        classes_of_derived = []
        element_class_data = self.db.mesh_object_classes()

        if all(
            [
                primal_class is None
                for primal_class in derived_spec["primals_class"]
            ]
        ):
            # CASE 1: All primals must exist for same element class as derived result
            for class_name, class_def in element_class_data.items():
                if "only_sclasses" in derived_spec:
                    if class_def.sclass not in derived_spec["only_sclasses"]:
                        continue
                primals_found = [
                    self.__variable_exists_for_class(primal, class_name)
                    for primal in derived_spec["primals"]
                ]
                if all(primals_found):
                    classes_of_derived.append(class_name)
        else:
            # CASE 2: primals must exists for class different from derived result
            for class_name, class_def in element_class_data.items():
                if "only_sclasses" in derived_spec:
                    if class_def.sclass not in derived_spec["only_sclasses"]:
                        continue
                primals_found = [
                    self.__variable_exists_for_class(primal, primal_class)
                    for primal, primal_class in zip(
                        derived_spec["primals"], derived_spec["primals_class"]
                    )
                ]
                if all(primals_found):
                    classes_of_derived.append(class_name)

        return classes_of_derived

    def find_batchable_queries(
        self, result_names: List[str]
    ) -> List[List[str]]:
        """Determine if any derived queries can be batched based on the result names."""
        groups = []
        result_names = sorted(result_names)
        for res in result_names:
            if res in self.__derived_expressions:
                groups.append(
                    (
                        res,
                        self.__derived_expressions[res][
                            "compute_function"
                        ].__name__,
                        self.__derived_expressions[res].get(
                            "supports_batching", False
                        ),
                    )
                )
        grouped_by_compute_function = [
            list(g) for _, g in groupby(groups, lambda x: x[1])
        ]
        final_groups: List[List[str]] = []
        for group in grouped_by_compute_function:
            if all(g[2] for g in group):
                final_groups.append([g[0] for g in group])
            else:
                for g in group:
                    final_groups.append([g[0]])
        return final_groups

    def query(self, *args: Any, **kwargs: Any) -> Any:
        """Derived *value* query — the next Phase-H sub-slice.

        Parity-sensitive value math (decision 19). Node displacement
        already routes through the Rust core (``mili_rs::derived``); the
        remaining families are not yet ported — raise explicitly rather
        than return a silent wrong answer.
        """
        return self.__value_not_ported()
