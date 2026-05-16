//! Phase-H derived-variable sub-slice — nodal displacement family.
//!
//! Bit-exact mirror of the upstream nodal-displacement derived
//! expressions (`reference/mili-python/src/mili/derived.py`):
//! `__compute_node_displacement` (~978), `__compute_node_displacement_
//! magnitude` (~1001), `__compute_node_radial_displacement` (~1023) and
//! the shared `__get_nodal_reference_positions` (~948).
//!
//! - `disp_<dir> = <ux|uy|uz> - reference`
//! - `disp_mag   = sqrt(dx^2 + dy^2 + dz^2)`
//! - `disp_rad_mag_xy = sqrt(dx^2 + dy^2)`
//!
//! where the per-node `reference` for direction `dir` is, at the
//! default `reference_state == 0`, the initial nodal coordinate
//! (`db.nodes()`) selected by the ordinals of the primal-returned node
//! labels within the node class's label list (upstream
//! `np.where(np.isin(labels_of_class, labels))[0]`); at a non-zero
//! `reference_state` it is the primal `u<dir>` value queried at that
//! state, aligned to the primal-returned labels.
//!
//! Velocities / accelerations (finite-difference over states) landed
//! in their own sub-slice; the scalar stress invariants (`pressure`,
//! `eff_stress`, `triaxiality`, `norm_press`) and the eigenvalue-based
//! principal-stress / principal-dev-stress / max-shear-stress family
//! are below (the latter on a symmetric-3x3 Jacobi eigensolver,
//! computed in f64 then cast to the primal dtype — bit-identical to
//! numpy's native `eigvalsh` at every literal-checked point). The
//! strain analogues (`vol_strain`, `prin_strain*`, `prin_dev_strain*`,
//! the `*_alt` griz closed-form variants) are a later sub-slice. The
//! reduction (`ResultModifier`) math is a
//! decision-18 Python-over-primal post-process and lives in `milox`,
//! not here.

use std::collections::HashMap;

use crate::error::{MiliError, Result};
use crate::query::{QueryResult, StateValues};

/// Resolve a node-displacement-component derived name to `(component
/// direction, title)`. `disp_x` → `(0, "X Displacement")`, etc.
pub fn node_disp_spec(name: &str) -> Option<(usize, &'static str)> {
    match name {
        "disp_x" => Some((0, "X Displacement")),
        "disp_y" => Some((1, "Y Displacement")),
        "disp_z" => Some((2, "Z Displacement")),
        _ => None,
    }
}

/// Resolve a node-displacement-magnitude derived name to `(component
/// directions, title)`. `disp_mag` → `([0,1,2], "Displacement
/// Magnitude")`, `disp_rad_mag_xy` → `([0,1], "Radial Displacement
/// Magnitude XY")` (`derived.py:78-94`).
pub fn node_disp_mag_spec(name: &str) -> Option<(&'static [usize], &'static str)> {
    match name {
        "disp_mag" => Some((&[0, 1, 2], "Displacement Magnitude")),
        "disp_rad_mag_xy" => Some((&[0, 1], "Radial Displacement Magnitude XY")),
        _ => None,
    }
}

/// The primal nodal-position state variable for direction `dir`:
/// `0 → ux`, `1 → uy`, `2 → uz` (`derived.py:59/67/74`).
pub fn node_disp_primal(dir: usize) -> &'static str {
    ["ux", "uy", "uz"][dir]
}

/// Per-primal-label reference value for direction `dir` from the
/// **initial** nodal coordinates (`reference_state == 0`;
/// `derived.py:962-966`).
///
/// `ordinals` are the ascending indices into `node_labels` whose label
/// appears in the primal-returned label set (upstream
/// `np.where(np.isin(labels_of_class, labels))[0]`), giving one
/// reference row per primal label row (both are ascending node-ordinal
/// order, so the rows align positionally).
pub fn nodal_reference_from_coords(
    primal_labels: &[i32],
    node_labels: &[i32],
    node_coords: &[f32],
    dims: usize,
    dir: usize,
) -> Result<Vec<f32>> {
    let queried: std::collections::HashSet<i32> = primal_labels.iter().copied().collect();
    let ordinals: Vec<usize> = node_labels
        .iter()
        .enumerate()
        .filter(|(_, l)| queried.contains(l))
        .map(|(i, _)| i)
        .collect();

    if ordinals.len() != primal_labels.len() {
        return Err(MiliError::Unsupported(
            "node-displacement reference/primal label count mismatch",
        ));
    }

    ordinals
        .iter()
        .map(|&ord| {
            node_coords
                .get(ord * dims + dir)
                .copied()
                .ok_or(MiliError::Unsupported(
                    "node-displacement reference index out of range",
                ))
        })
        .collect()
}

/// Per-primal-label reference value from a single-state primal
/// `u<dir>` query at a non-zero `reference_state`
/// (`derived.py:967-974`). `ref_query` carries one value per label;
/// align it to the primal-returned labels by label id (both queries
/// pass the same label set, so every primal label is present).
pub fn nodal_reference_from_query(
    primal_labels: &[i32],
    ref_query: &QueryResult,
) -> Result<Vec<f32>> {
    let StateValues::F32(vals) = &ref_query.values else {
        return Err(MiliError::Unsupported(
            "node displacement requires float32 nodal position primals",
        ));
    };
    if vals.len() != ref_query.labels.len() {
        return Err(MiliError::Unsupported(
            "node-displacement reference query is not a single scalar state",
        ));
    }
    let by_label: HashMap<i32, f32> = ref_query
        .labels
        .iter()
        .copied()
        .zip(vals.iter().copied())
        .collect();
    primal_labels
        .iter()
        .map(|l| {
            by_label.get(l).copied().ok_or(MiliError::Unsupported(
                "node-displacement reference query missing a primal label",
            ))
        })
        .collect()
}

/// `disp_<dir> = primal - reference`, broadcast over states.
///
/// `primal` is `[state][label]` row-major; `reference` is one value
/// per label (already aligned to `primal.labels`), repeated every
/// state — `cycle()` aligns it with each state block (upstream numpy
/// broadcast `primal_data['data'] - reference_data`).
pub fn compute_node_displacement(
    primal: QueryResult,
    reference: &[f32],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let StateValues::F32(prim) = primal.values else {
        return Err(MiliError::Unsupported(
            "node displacement requires float32 nodal position primals",
        ));
    };
    if reference.len() != primal.labels.len() {
        return Err(MiliError::Unsupported(
            "node-displacement reference/primal label count mismatch",
        ));
    }

    let disp: Vec<f32> = if primal.labels.is_empty() {
        Vec::new()
    } else {
        prim.iter()
            .zip(reference.iter().cycle())
            .map(|(p, r)| p - r)
            .collect()
    };

    Ok(QueryResult {
        values: StateValues::F32(disp),
        labels: primal.labels,
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: primal.class_name,
    })
}

/// `disp_mag = sqrt(sum_d (u_d - ref_d)^2)` over the requested
/// directions (`derived.py:1014-1019` / `1036-1040`).
///
/// `primals[i]` is the `u<dirs[i]>` query `[state][label]`;
/// `references[i]` is the per-label reference for that direction.
/// All primals share the same `labels` / state axis / class (one
/// `query()` per component on the same args).
pub fn compute_node_displacement_magnitude(
    primals: &[QueryResult],
    references: &[Vec<f32>],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let first = primals.first().ok_or(MiliError::Unsupported(
        "displacement magnitude requires at least one component primal",
    ))?;
    let n = first.values.len();
    let labels = first.labels.clone();

    let mut comps: Vec<&[f32]> = Vec::with_capacity(primals.len());
    for p in primals {
        let StateValues::F32(v) = &p.values else {
            return Err(MiliError::Unsupported(
                "node displacement requires float32 nodal position primals",
            ));
        };
        if v.len() != n || p.labels.len() != labels.len() {
            return Err(MiliError::Unsupported(
                "displacement magnitude component primals disagree in shape",
            ));
        }
        comps.push(v.as_slice());
    }
    if references.len() != comps.len() {
        return Err(MiliError::Unsupported(
            "displacement magnitude reference/component count mismatch",
        ));
    }
    for r in references {
        if r.len() != labels.len() {
            return Err(MiliError::Unsupported(
                "node-displacement reference/primal label count mismatch",
            ));
        }
    }

    let n_lab = labels.len();
    let mag: Vec<f32> = if n_lab == 0 {
        Vec::new()
    } else {
        (0..n)
            .map(|i| {
                let k = i % n_lab; // reference repeats every state block
                let mut acc = 0.0f32;
                for (c, r) in comps.iter().zip(references.iter()) {
                    let d = c[i] - r[k];
                    acc += d * d;
                }
                acc.sqrt()
            })
            .collect()
    };

    Ok(QueryResult {
        values: StateValues::F32(mag),
        labels,
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: first.class_name.clone(),
    })
}

/// Resolve a nodal-velocity derived name to `(direction, title)`
/// (`derived.py:95-115`).
pub fn node_vel_spec(name: &str) -> Option<(usize, &'static str)> {
    match name {
        "vel_x" => Some((0, "X Velocity")),
        "vel_y" => Some((1, "Y Velocity")),
        "vel_z" => Some((2, "Z Velocity")),
        _ => None,
    }
}

/// Resolve a nodal-acceleration derived name to `(direction, title)`
/// (`derived.py:116-136`).
pub fn node_acc_spec(name: &str) -> Option<(usize, &'static str)> {
    match name {
        "acc_x" => Some((0, "X Acceleration")),
        "acc_y" => Some((1, "Y Acceleration")),
        "acc_z" => Some((2, "Z Acceleration")),
        _ => None,
    }
}

/// Time of 1-based state `n`: upstream `db.times()[n-1]` (the f32
/// per-state header time).
fn state_time(times: &[f32], n: i64) -> Result<f32> {
    usize::try_from(n - 1)
        .ok()
        .and_then(|i| times.get(i).copied())
        .ok_or(MiliError::Unsupported(
            "nodal kinematics: required state time out of range",
        ))
}

/// `[label]` row for 1-based state `n` from the gathered
/// `[state][label]` primal, located via `gathered_state_nums`.
fn gathered_row<'a, T>(
    vals: &'a [T],
    gathered_state_nums: &[i64],
    n_lab: usize,
    n: i64,
) -> Result<&'a [T]> {
    let r = gathered_state_nums
        .iter()
        .position(|&g| g == n)
        .ok_or(MiliError::Unsupported(
            "nodal kinematics: required state not gathered",
        ))?;
    Ok(&vals[r * n_lab..r * n_lab + n_lab])
}

// The nodal velocity / acceleration finite-difference math is identical
// for f32 (single-precision plt) and f64 (double-precision plt) primals
// except the dtype the per-label arithmetic runs in. The time-derived
// factor is computed in f32 (upstream `times()` is the f32 per-state
// header; numpy NEP50 keeps `1.0 / f32` and `0.5 * f32` in f32), then
// promoted to the primal dtype for the final multiply — exactly
// numpy's `f64_disp * f32_factor -> f64` / `f32_disp * f32_factor ->
// f32` promotion. This macro generates the two concrete bodies.
macro_rules! impl_kinematics {
    ($vel_fn:ident, $acc_fn:ident, $t:ty) => {
        fn $vel_fn(
            vals: &[$t],
            gsn: &[i64],
            req: &[i64],
            times: &[f32],
            n_lab: usize,
        ) -> Result<Vec<$t>> {
            let mut out: Vec<$t> = Vec::with_capacity(req.len() * n_lab);
            for &s in req {
                if s == 1 {
                    // Velocity at the first state is defined zero
                    // (`derived.py:1062`).
                    out.extend(std::iter::repeat(<$t>::default()).take(n_lab));
                    continue;
                }
                let cur = gathered_row(vals, gsn, n_lab, s)?;
                let prev = gathered_row(vals, gsn, n_lab, s - 1)?;
                let dt = state_time(times, s)? - state_time(times, s - 1)?;
                let inv = 1.0f32 / dt;
                for l in 0..n_lab {
                    out.push((cur[l] - prev[l]) * (inv as $t));
                }
            }
            Ok(out)
        }

        fn $acc_fn(
            vals: &[$t],
            gsn: &[i64],
            req: &[i64],
            times: &[f32],
            max_state: i64,
            n_lab: usize,
        ) -> Result<Vec<$t>> {
            let mut out: Vec<$t> = Vec::with_capacity(req.len() * n_lab);
            for &s in req {
                // (a, b, c, dt) so accel = (a - 2*b + c) / dt^2, with
                // the three component rows picked per the central /
                // forward / backward stencil (`derived.py:1117-1153`).
                let (a, b, c, dt) = if s == 1 {
                    // Forward difference: u(3) - 2 u(2) + u(1).
                    let dt = 0.5f32 * (state_time(times, 3)? - state_time(times, 1)?);
                    (
                        gathered_row(vals, gsn, n_lab, 3)?,
                        gathered_row(vals, gsn, n_lab, 2)?,
                        gathered_row(vals, gsn, n_lab, 1)?,
                        dt,
                    )
                } else if s == max_state {
                    // Backward difference: u(N) - 2 u(N-1) + u(N-2).
                    let dt = 0.5f32
                        * (state_time(times, max_state)? - state_time(times, max_state - 2)?);
                    (
                        gathered_row(vals, gsn, n_lab, max_state)?,
                        gathered_row(vals, gsn, n_lab, max_state - 1)?,
                        gathered_row(vals, gsn, n_lab, max_state - 2)?,
                        dt,
                    )
                } else {
                    // Central difference: u(s+1) - 2 u(s) + u(s-1).
                    let dt = 0.5f32 * (state_time(times, s + 1)? - state_time(times, s - 1)?);
                    (
                        gathered_row(vals, gsn, n_lab, s + 1)?,
                        gathered_row(vals, gsn, n_lab, s)?,
                        gathered_row(vals, gsn, n_lab, s - 1)?,
                        dt,
                    )
                };
                let ot = 1.0f32 / (dt * dt);
                for l in 0..n_lab {
                    // `2*b` as `b+b`: exact in IEEE (doubling never
                    // rounds), bit-identical to numpy's `2*u_c`, and
                    // avoids an `i32 as $t` precision-loss cast.
                    out.push((a[l] - (b[l] + b[l]) + c[l]) * (ot as $t));
                }
            }
            Ok(out)
        }
    };
}

impl_kinematics!(vel_f32, acc_f32, f32);
impl_kinematics!(vel_f64, acc_f64, f64);

/// `vel_<dir> = (u(s) - u(s-1)) / (t(s) - t(s-1))`, zero at state 1
/// (`derived.py.__compute_node_velocity`).
///
/// `gathered` is the primal `u<dir>` queried at every state the
/// stencil needs (`gathered_state_nums`, parallel to its state axis);
/// the result is one row per `requested_state_nums` entry.
pub fn compute_node_velocity(
    gathered: QueryResult,
    gathered_state_nums: &[i64],
    requested_state_nums: &[i64],
    times: &[f32],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let n_lab = gathered.labels.len();
    let values = match &gathered.values {
        StateValues::F32(v) => StateValues::F32(vel_f32(
            v,
            gathered_state_nums,
            requested_state_nums,
            times,
            n_lab,
        )?),
        StateValues::F64(v) => StateValues::F64(vel_f64(
            v,
            gathered_state_nums,
            requested_state_nums,
            times,
            n_lab,
        )?),
        _ => {
            return Err(MiliError::Unsupported(
                "nodal velocity requires float nodal position primals",
            ))
        }
    };
    Ok(QueryResult {
        values,
        labels: gathered.labels,
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: gathered.class_name,
    })
}

/// `acc_<dir>` via central difference, with forward/backward stencils
/// at the first/last state (`derived.py.__compute_node_acceleration`).
pub fn compute_node_acceleration(
    gathered: QueryResult,
    gathered_state_nums: &[i64],
    requested_state_nums: &[i64],
    times: &[f32],
    max_state: i64,
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let n_lab = gathered.labels.len();
    let values = match &gathered.values {
        StateValues::F32(v) => StateValues::F32(acc_f32(
            v,
            gathered_state_nums,
            requested_state_nums,
            times,
            max_state,
            n_lab,
        )?),
        StateValues::F64(v) => StateValues::F64(acc_f64(
            v,
            gathered_state_nums,
            requested_state_nums,
            times,
            max_state,
            n_lab,
        )?),
        _ => {
            return Err(MiliError::Unsupported(
                "nodal acceleration requires float nodal position primals",
            ))
        }
    };
    Ok(QueryResult {
        values,
        labels: gathered.labels,
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: gathered.class_name,
    })
}

/// The scalar (non-eigenvalue) stress invariants
/// (`derived.py.__compute_{pressure,effective_stress,triaxiality,
/// normalized_pressure}` ~1467-1671). All are pure element-wise
/// arithmetic over the 6 stress component primals on the requested
/// element class (`sx/sy/sz/sxy/syz/szx`; `pressure` needs only the
/// three normals) — no `np.linalg.eigvalsh`, so no eigensolver. The
/// principal-stress / principal-dev-stress / max-shear-stress family
/// (and the strain analogues) are a later sub-slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StressInvariant {
    Pressure,
    EffStress,
    Triaxiality,
    NormPress,
}

/// Resolve a stress-invariant derived name to `(kind, title)`
/// (`derived.py` `__derived_expressions` table, ~339-419).
pub fn stress_invariant_spec(name: &str) -> Option<(StressInvariant, &'static str)> {
    match name {
        "pressure" => Some((StressInvariant::Pressure, "Pressure")),
        "eff_stress" => Some((StressInvariant::EffStress, "Effective Stress")),
        "triaxiality" => Some((StressInvariant::Triaxiality, "Triaxiality")),
        "norm_press" => Some((StressInvariant::NormPress, "Normalized Pressure")),
        _ => None,
    }
}

/// The primal component svars this invariant reads, in the order the
/// compute kernel indexes them. `pressure` uses only the three normal
/// stresses (`derived.py:1520-1523`); the rest use all six.
pub fn stress_invariant_primals(inv: StressInvariant) -> &'static [&'static str] {
    match inv {
        StressInvariant::Pressure => &["sx", "sy", "sz"],
        _ => &["sx", "sy", "sz", "sxy", "syz", "szx"],
    }
}

// The invariant math is identical for f32 (single-precision plt) and
// f64 (double-precision plt) primals except the dtype the element-wise
// arithmetic runs in. Mirrors numpy NEP50 exactly: the Python-float
// `-(1/3)` / `0.5` / int `3` are weak scalars, so the operation stays
// in the array (primal) dtype with the scalar cast to that dtype —
// e.g. `f32(-(1/3)) * f32_array` (the established f32-per-state-time
// NEP50 lesson, here for pure component arithmetic). `x**2` is numpy's
// `fast_scalar_power` short-circuit to `x*x` (exact). Op order is
// left-associative exactly as the Python source evaluates it. This
// macro generates the two concrete kernels.
macro_rules! impl_stress_invariant {
    ($fn:ident, $t:ty) => {
        fn $fn(inv: StressInvariant, c: &[&[$t]]) -> Vec<$t> {
            let n = c[0].len();
            // numpy: the Python float `-(1/3)` is a weak scalar cast to
            // the array dtype, so the constant is the array-dtype
            // rounding of 1/3 — `-1.0/3.0` evaluated in `$t` is exactly
            // that (f32: the correctly-rounded f32 reciprocal, same bit
            // pattern as f64(-1/3) cast to f32; f64: -0.333…3 as in
            // Python). No `as` cast → no precision-loss / truncation.
            let third: $t = -1.0 / 3.0;
            let half: $t = 0.5;
            let three: $t = 3.0;
            let (sx, sy, sz) = (c[0], c[1], c[2]);
            if let StressInvariant::Pressure = inv {
                // `(-1/3) * (sx + sy + sz)` (`derived.py:1523`).
                return (0..n).map(|i| third * ((sx[i] + sy[i]) + sz[i])).collect();
            }
            let (sxy, syz, szx) = (c[3], c[4], c[5]);
            // `eff_stress` only: if every sx==sy and sy==sz to within
            // atol=1e-15 (rtol=0) over the *whole* queried array, the
            // pressure collapses to `-sx` to kill the round-off in
            // `-(1/3)*(sx+sy+sz)` for hydrostatic states
            // (`derived.py:1491-1496`). `np.allclose` is a whole-array
            // reduction; `and` short-circuits.
            let hydro = matches!(inv, StressInvariant::EffStress) && {
                let atol: $t = 1e-15;
                (0..n).all(|i| (sx[i] - sy[i]).abs() <= atol)
                    && (0..n).all(|i| (sy[i] - sz[i]).abs() <= atol)
            };
            (0..n)
                .map(|i| {
                    let p: $t = if hydro {
                        -sx[i]
                    } else {
                        third * ((sx[i] + sy[i]) + sz[i])
                    };
                    let dx = sx[i] + p;
                    let dy = sy[i] + p;
                    let dz = sz[i] + p;
                    // J2 = 0.5*(dx^2+dy^2+dz^2) + sxy^2 + syz^2 + szx^2,
                    // left-associative exactly as Python evaluates it
                    // (`derived.py:1503-1504` / 1631-1632 / 1663-1664).
                    let j2 = half * (((dx * dx) + (dy * dy)) + (dz * dz))
                        + sxy[i] * sxy[i]
                        + syz[i] * syz[i]
                        + szx[i] * szx[i];
                    let seff = (three * j2).sqrt();
                    match inv {
                        StressInvariant::EffStress => seff,
                        StressInvariant::Triaxiality => -p / seff,
                        StressInvariant::NormPress => p / seff,
                        StressInvariant::Pressure => unreachable!(),
                    }
                })
                .collect()
        }
    };
}

impl_stress_invariant!(stress_invariant_f32, f32);
impl_stress_invariant!(stress_invariant_f64, f64);

/// Compute a scalar stress invariant from its component primals.
///
/// `primals` are the `stress_invariant_primals(inv)` queries (same
/// class / labels / states / ips, so identical shape and flat
/// `[state][label][atom]` length — upstream broadcasts them
/// element-wise). The result keeps the primal's entity/atom axes; the
/// `components` axis is the single derived name (upstream
/// `__initialize_result_dictionary` sets `components = [result_name]`
/// while `data` keeps `np.empty_like(primal)`'s shape — the binding
/// derives the atom count from the flat length, not `components`).
pub fn compute_stress_invariant(
    inv: StressInvariant,
    primals: &[QueryResult],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let need = stress_invariant_primals(inv).len();
    if primals.len() != need {
        return Err(MiliError::Unsupported(
            "stress invariant primal count mismatch",
        ));
    }
    let first = &primals[0];
    let n = first.values.len();
    for p in primals {
        if p.values.len() != n || p.labels.len() != first.labels.len() {
            return Err(MiliError::Unsupported(
                "stress invariant component primals disagree in shape",
            ));
        }
    }

    let values = match &first.values {
        StateValues::F32(_) => {
            let mut cols: Vec<&[f32]> = Vec::with_capacity(need);
            for p in primals {
                let StateValues::F32(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "stress invariant component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F32(stress_invariant_f32(inv, &cols))
        }
        StateValues::F64(_) => {
            let mut cols: Vec<&[f64]> = Vec::with_capacity(need);
            for p in primals {
                let StateValues::F64(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "stress invariant component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F64(stress_invariant_f64(inv, &cols))
        }
        _ => {
            return Err(MiliError::Unsupported(
                "stress invariants require float stress primals",
            ))
        }
    };

    Ok(QueryResult {
        values,
        labels: first.labels.clone(),
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: first.class_name.clone(),
    })
}

/// Ascending eigenvalues of a symmetric 3x3 matrix via classic cyclic
/// Jacobi rotations, all in f64.
///
/// Upstream computes `prin_stress*` / `prin_dev_stress*` /
/// `max_shear_stress` with `np.linalg.eigvalsh` (LAPACK `?syevd`) on
/// the primal-dtype matrix. LAPACK's exact bits are
/// implementation-defined and not reproducible from a hand-written
/// solver, **but** for these well-conditioned 3x3 stress matrices the
/// eigenvalues computed in f64 and rounded to f32 are bit-identical to
/// numpy's native f32 `eigvalsh` at every literal-checked test point
/// (empirically verified across the brick/shell/beam corpus, all IP
/// configs; the loose `test_derived` deltas — written to absorb the
/// Griz-C vs numpy spread — leave ample margin). So: assemble the
/// matrix in the primal dtype (numpy's matrix entries), promote to
/// f64, eigensolve here, cast the eigenvalues back to the primal
/// dtype. The rotation/threshold/sort schedule is fixed (50 sweeps,
/// `1e-300` off-diagonal cutoff, `copysign` tangent) — that exact
/// schedule is what was validated bit-for-bit against the oracle.
fn jacobi_eigvalsh_sym3(mut a: [[f64; 3]; 3]) -> [f64; 3] {
    for _ in 0..50 {
        if a[0][1].abs() + a[0][2].abs() + a[1][2].abs() < 1e-300 {
            break;
        }
        for &(p, q) in &[(0usize, 1usize), (0, 2), (1, 2)] {
            let apq = a[p][q];
            if apq == 0.0 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
            // Python special-cases `th == 0.0` to `t = 1.0` (also picks
            // up -0.0, which `copysign` would otherwise send to -1.0).
            let t = if theta == 0.0 {
                1.0
            } else {
                1.0_f64.copysign(theta) / (theta.abs() + (theta * theta + 1.0).sqrt())
            };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            // Column then row rotation, unrolled over k = 0..3 (the
            // 2-D mixed indexing isn't a plain range-loop pattern).
            for k in [0usize, 1, 2] {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in [0usize, 1, 2] {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
        }
    }
    let mut ev = [a[0][0], a[1][1], a[2][2]];
    // Python `sorted(...)` ascending; the stress data is finite so the
    // total order is well-defined (the `unwrap_or` never triggers).
    ev.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    ev
}

/// The eigenvalue-based stress invariants
/// (`derived.py.__compute_{principal_stress,principal_dev_stress,
/// max_shear_stress}` ~1422-1607). Each builds the symmetric stress
/// (or deviatoric) 3x3 and reads `eigvalsh`; the strain analogues are
/// a later sub-slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrincipalStress {
    Prin1,
    Prin2,
    Prin3,
    Dev1,
    Dev2,
    Dev3,
    MaxShear,
}

/// Resolve an eigenvalue-stress derived name to `(kind, title)`
/// (`derived.py` `__derived_expressions` table, ~291-395).
pub fn principal_stress_spec(name: &str) -> Option<(PrincipalStress, &'static str)> {
    match name {
        "prin_stress1" => Some((PrincipalStress::Prin1, "Principal Stress 1")),
        "prin_stress2" => Some((PrincipalStress::Prin2, "Principal Stress 2")),
        "prin_stress3" => Some((PrincipalStress::Prin3, "Principal Stress 3")),
        "prin_dev_stress1" => Some((PrincipalStress::Dev1, "Principal Deviatoric Stress 1")),
        "prin_dev_stress2" => Some((PrincipalStress::Dev2, "Principal Deviatoric Stress 2")),
        "prin_dev_stress3" => Some((PrincipalStress::Dev3, "Principal Deviatoric Stress 3")),
        "max_shear_stress" => Some((PrincipalStress::MaxShear, "Maximum Shear Stress")),
        _ => None,
    }
}

/// The 6 stress component primals every eigenvalue-stress invariant
/// reads, in the order the kernel indexes them.
pub fn principal_stress_primals() -> &'static [&'static str] {
    &["sx", "sy", "sz", "sxy", "syz", "szx"]
}

// Generic over f32 / f64 primals (mirrors `impl_stress_invariant`):
// the deviatoric pressure (`prin_dev_stress*` / `max_shear_stress`)
// and the `0.5*(max-min)` shear are computed in the primal dtype with
// numpy NEP50 weak-scalar promotion (type-annotated `$t` literals);
// the symmetric matrix is promoted to f64 for the shared eigensolver,
// and the eigenvalues cast back to the primal dtype exactly as numpy's
// f32 `eigvalsh` would yield (verified bit-for-bit vs the oracle).
macro_rules! impl_principal_stress {
    ($fn:ident, $t:ty) => {
        fn $fn(kind: PrincipalStress, c: &[&[$t]]) -> Vec<$t> {
            let n = c[0].len();
            // numpy weak-scalar `-(1/3)` / `0.5` cast to the array
            // dtype (see `impl_stress_invariant`).
            let third: $t = -1.0 / 3.0;
            let half: $t = 0.5;
            let (sx, sy, sz, sxy, syz, szx) = (c[0], c[1], c[2], c[3], c[4], c[5]);
            // `prin_dev_stress*` and `max_shear_stress` eigensolve the
            // *deviatoric* matrix (diagonal shifted by the pressure);
            // `prin_stress*` use the raw stress matrix
            // (`derived.py:1543-1549` / 1589-1595 vs 1439-1442).
            let dev = !matches!(
                kind,
                PrincipalStress::Prin1 | PrincipalStress::Prin2 | PrincipalStress::Prin3
            );
            (0..n)
                .map(|i| {
                    let (dxx, dyy, dzz) = if dev {
                        let p = third * ((sx[i] + sy[i]) + sz[i]);
                        (sx[i] + p, sy[i] + p, sz[i] + p)
                    } else {
                        (sx[i], sy[i], sz[i])
                    };
                    // Symmetric matrix [[dxx,sxy,szx],[sxy,dyy,syz],
                    // [szx,syz,dzz]] (`np.stack` columns,
                    // `derived.py:1439-1442`), promoted to f64.
                    let m = [
                        [dxx as f64, sxy[i] as f64, szx[i] as f64],
                        [sxy[i] as f64, dyy as f64, syz[i] as f64],
                        [szx[i] as f64, syz[i] as f64, dzz as f64],
                    ];
                    let ev = jacobi_eigvalsh_sym3(m); // ascending
                    let mn = ev[0] as $t;
                    let md = ev[1] as $t;
                    let mx = ev[2] as $t;
                    match kind {
                        // eigvalsh ascending: max = [...]1, mid = [...]2,
                        // min = [...]3 (`derived.py:1454-1459`).
                        PrincipalStress::Prin1 | PrincipalStress::Dev1 => mx,
                        PrincipalStress::Prin2 | PrincipalStress::Dev2 => md,
                        PrincipalStress::Prin3 | PrincipalStress::Dev3 => mn,
                        // `0.5*(max-min)` in the primal dtype, exactly
                        // as numpy reduces the f32 eigenvalues
                        // (`derived.py:1603`).
                        PrincipalStress::MaxShear => half * (mx - mn),
                    }
                })
                .collect()
        }
    };
}

impl_principal_stress!(principal_stress_f32, f32);
impl_principal_stress!(principal_stress_f64, f64);

/// Compute an eigenvalue-based stress invariant from its 6 component
/// primals (same shape contract as [`compute_stress_invariant`]).
pub fn compute_principal_stress(
    kind: PrincipalStress,
    primals: &[QueryResult],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    if primals.len() != 6 {
        return Err(MiliError::Unsupported(
            "principal stress primal count mismatch",
        ));
    }
    let first = &primals[0];
    let n = first.values.len();
    for p in primals {
        if p.values.len() != n || p.labels.len() != first.labels.len() {
            return Err(MiliError::Unsupported(
                "principal stress component primals disagree in shape",
            ));
        }
    }

    let values = match &first.values {
        StateValues::F32(_) => {
            let mut cols: Vec<&[f32]> = Vec::with_capacity(6);
            for p in primals {
                let StateValues::F32(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "principal stress component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F32(principal_stress_f32(kind, &cols))
        }
        StateValues::F64(_) => {
            let mut cols: Vec<&[f64]> = Vec::with_capacity(6);
            for p in primals {
                let StateValues::F64(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "principal stress component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F64(principal_stress_f64(kind, &cols))
        }
        _ => {
            return Err(MiliError::Unsupported(
                "principal stress requires float stress primals",
            ))
        }
    };

    Ok(QueryResult {
        values,
        labels: first.labels.clone(),
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: first.class_name.clone(),
    })
}

/// The non-`*_alt` strain invariants
/// (`derived.py.__compute_{vol_strain,principal_strain,
/// dev_principal_strain}` ~1157-1340). `vol_strain` is the trivial
/// strain trace `ex+ey+ez`; the principal / principal-deviatoric
/// strains reuse the same symmetric-3x3 Jacobi eigensolver as the
/// stress family ([`jacobi_eigvalsh_sym3`]) on the 6 strain
/// components. The `*_alt` griz closed-form trig variants are a
/// distinct algorithm — a later sub-slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrincipalStrain {
    Vol,
    Prin1,
    Prin2,
    Prin3,
    Dev1,
    Dev2,
    Dev3,
}

/// Resolve a strain-invariant derived name to `(kind, title)`
/// (`derived.py` `__derived_expressions` table, ~137-217).
pub fn principal_strain_spec(name: &str) -> Option<(PrincipalStrain, &'static str)> {
    match name {
        "vol_strain" => Some((PrincipalStrain::Vol, "Volumetric Strain")),
        "prin_strain1" => Some((PrincipalStrain::Prin1, "Principal Strain 1")),
        "prin_strain2" => Some((PrincipalStrain::Prin2, "Principal Strain 2")),
        "prin_strain3" => Some((PrincipalStrain::Prin3, "Principal Strain 3")),
        "prin_dev_strain1" => Some((PrincipalStrain::Dev1, "Principal Deviatoric Strain 1")),
        "prin_dev_strain2" => Some((PrincipalStrain::Dev2, "Principal Deviatoric Strain 2")),
        "prin_dev_strain3" => Some((PrincipalStrain::Dev3, "Principal Deviatoric Strain 3")),
        _ => None,
    }
}

/// The strain component primals this invariant reads, in kernel order.
/// `vol_strain` needs only the three normal strains
/// (`derived.py:1167-1169`); the principal strains need all six.
pub fn principal_strain_primals(kind: PrincipalStrain) -> &'static [&'static str] {
    match kind {
        PrincipalStrain::Vol => &["ex", "ey", "ez"],
        _ => &["ex", "ey", "ez", "exy", "eyz", "ezx"],
    }
}

// Generic over f32 / f64 (mirrors `impl_principal_stress`). The
// hydrostatic strain `e_hyd = (1/3)*(ex+ey+ez)` (note: *positive*
// 1/3, and the deviatoric diagonal is `ex - e_hyd`, vs stress's
// `p = -(1/3)*sum; sx + p` — algebraically and bit-for-bit the same
// `component - (1/3)*trace`, but the source spells it this way for
// strain, so we mirror that spelling) is computed in the primal dtype
// with numpy NEP50 weak-scalar promotion (type-annotated `$t`
// literal); the matrix is promoted to f64 for the shared eigensolver
// and the eigenvalues cast back, exactly as the stress family.
macro_rules! impl_principal_strain {
    ($fn:ident, $t:ty) => {
        fn $fn(kind: PrincipalStrain, c: &[&[$t]]) -> Vec<$t> {
            let n = c[0].len();
            let (ex, ey, ez) = (c[0], c[1], c[2]);
            if let PrincipalStrain::Vol = kind {
                // `ex + ey + ez` (`derived.py:1170`).
                return (0..n).map(|i| (ex[i] + ey[i]) + ez[i]).collect();
            }
            // numpy weak-scalar `(1/3)` cast to the array dtype.
            let e_third: $t = 1.0 / 3.0;
            let (exy, eyz, ezx) = (c[3], c[4], c[5]);
            let dev = !matches!(
                kind,
                PrincipalStrain::Prin1 | PrincipalStrain::Prin2 | PrincipalStrain::Prin3
            );
            (0..n)
                .map(|i| {
                    let (dxx, dyy, dzz) = if dev {
                        // e_hyd = (1/3)*(ex+ey+ez); diag = e - e_hyd
                        // (`derived.py:1312-1317`).
                        let eh = e_third * ((ex[i] + ey[i]) + ez[i]);
                        (ex[i] - eh, ey[i] - eh, ez[i] - eh)
                    } else {
                        (ex[i], ey[i], ez[i])
                    };
                    // Symmetric matrix [[dxx,exy,ezx],[exy,dyy,eyz],
                    // [ezx,eyz,dzz]] (`derived.py:1192-1195`),
                    // promoted to f64.
                    let m = [
                        [dxx as f64, exy[i] as f64, ezx[i] as f64],
                        [exy[i] as f64, dyy as f64, eyz[i] as f64],
                        [ezx[i] as f64, eyz[i] as f64, dzz as f64],
                    ];
                    let ev = jacobi_eigvalsh_sym3(m); // ascending
                    let mn = ev[0] as $t;
                    let md = ev[1] as $t;
                    let mx = ev[2] as $t;
                    match kind {
                        // eigvalsh ascending: max = [...]1, mid = [...]2,
                        // min = [...]3 (`derived.py:1207-1212`).
                        PrincipalStrain::Prin1 | PrincipalStrain::Dev1 => mx,
                        PrincipalStrain::Prin2 | PrincipalStrain::Dev2 => md,
                        PrincipalStrain::Prin3 | PrincipalStrain::Dev3 => mn,
                        PrincipalStrain::Vol => unreachable!(),
                    }
                })
                .collect()
        }
    };
}

impl_principal_strain!(principal_strain_f32, f32);
impl_principal_strain!(principal_strain_f64, f64);

/// Compute a strain invariant from its component primals (same shape
/// contract as [`compute_stress_invariant`]).
pub fn compute_principal_strain(
    kind: PrincipalStrain,
    primals: &[QueryResult],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let need = principal_strain_primals(kind).len();
    if primals.len() != need {
        return Err(MiliError::Unsupported(
            "principal strain primal count mismatch",
        ));
    }
    let first = &primals[0];
    let n = first.values.len();
    for p in primals {
        if p.values.len() != n || p.labels.len() != first.labels.len() {
            return Err(MiliError::Unsupported(
                "principal strain component primals disagree in shape",
            ));
        }
    }

    let values = match &first.values {
        StateValues::F32(_) => {
            let mut cols: Vec<&[f32]> = Vec::with_capacity(need);
            for p in primals {
                let StateValues::F32(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "principal strain component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F32(principal_strain_f32(kind, &cols))
        }
        StateValues::F64(_) => {
            let mut cols: Vec<&[f64]> = Vec::with_capacity(need);
            for p in primals {
                let StateValues::F64(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "principal strain component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F64(principal_strain_f64(kind, &cols))
        }
        _ => {
            return Err(MiliError::Unsupported(
                "principal strain requires float strain primals",
            ))
        }
    };

    Ok(QueryResult {
        values,
        labels: first.labels.clone(),
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: first.class_name.clone(),
    })
}

/// The remaining sqrt-of-sum-of-component-squares magnitude derived
/// (`derived.py.__compute_{nodal_tangential_traction_magnitude,
/// shear_magnitude}` ~1742 / ~2427) — the same element-wise pattern as
/// `disp_mag` / `eff_stress`, no connectivity / projection /
/// cross-derived dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagnitudeDerived {
    /// `nodtangmag = sqrt(nodtang_x^2 + nodtang_y^2 + nodtang_z^2)`.
    Nodtang,
    /// `shear_magnitude = sqrt(qxx^2 + qyy^2)`.
    Shear,
}

/// Resolve a magnitude derived name to `(kind, title)` (`derived.py`
/// `__derived_expressions` table, ~427 / ~593).
pub fn magnitude_spec(name: &str) -> Option<(MagnitudeDerived, &'static str)> {
    match name {
        "nodtangmag" => Some((
            MagnitudeDerived::Nodtang,
            "Nodal Tangential Traction Magnitude",
        )),
        "shear_magnitude" => Some((MagnitudeDerived::Shear, "Shear Magnitude")),
        _ => None,
    }
}

/// The component primals this magnitude reads, in kernel order.
pub fn magnitude_primals(kind: MagnitudeDerived) -> &'static [&'static str] {
    match kind {
        MagnitudeDerived::Nodtang => &["nodtang_x", "nodtang_y", "nodtang_z"],
        MagnitudeDerived::Shear => &["qxx", "qyy"],
    }
}

// `sqrt(sum_i c_i^2)` with the squares summed left-associatively
// exactly as the Python source spells it (`sqrt(((x*x + y*y) + z*z))`
// / `sqrt((qxx*qxx + qyy*qyy))`); `**2` is numpy `fast_scalar_power`
// -> `x*x`. Pure primal-dtype arithmetic (generic f32/f64), no scalar
// constants so no NEP50 promotion to track.
macro_rules! impl_magnitude {
    ($fn:ident, $t:ty) => {
        fn $fn(c: &[&[$t]]) -> Vec<$t> {
            let n = c[0].len();
            (0..n)
                .map(|i| {
                    let mut acc = c[0][i] * c[0][i];
                    for col in &c[1..] {
                        acc += col[i] * col[i];
                    }
                    acc.sqrt()
                })
                .collect()
        }
    };
}

impl_magnitude!(magnitude_f32, f32);
impl_magnitude!(magnitude_f64, f64);

/// Compute a magnitude derived from its component primals (same shape
/// contract as [`compute_stress_invariant`]).
pub fn compute_magnitude(
    kind: MagnitudeDerived,
    primals: &[QueryResult],
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let need = magnitude_primals(kind).len();
    if primals.len() != need {
        return Err(MiliError::Unsupported("magnitude primal count mismatch"));
    }
    let first = &primals[0];
    let n = first.values.len();
    for p in primals {
        if p.values.len() != n || p.labels.len() != first.labels.len() {
            return Err(MiliError::Unsupported(
                "magnitude component primals disagree in shape",
            ));
        }
    }

    let values = match &first.values {
        StateValues::F32(_) => {
            let mut cols: Vec<&[f32]> = Vec::with_capacity(need);
            for p in primals {
                let StateValues::F32(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "magnitude component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F32(magnitude_f32(&cols))
        }
        StateValues::F64(_) => {
            let mut cols: Vec<&[f64]> = Vec::with_capacity(need);
            for p in primals {
                let StateValues::F64(v) = &p.values else {
                    return Err(MiliError::Unsupported(
                        "magnitude component primals disagree in dtype",
                    ));
                };
                cols.push(v.as_slice());
            }
            StateValues::F64(magnitude_f64(&cols))
        }
        _ => {
            return Err(MiliError::Unsupported(
                "magnitude requires float component primals",
            ))
        }
    };

    Ok(QueryResult {
        values,
        labels: first.labels.clone(),
        components: vec![result_name.to_owned()],
        title: title.to_owned(),
        class_name: first.class_name.clone(),
    })
}
