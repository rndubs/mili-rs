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
//! Velocities / accelerations (finite-difference over states) and the
//! stress/strain invariants are later sub-slices. The reduction
//! (`ResultModifier`) math is a decision-18 Python-over-primal
//! post-process and lives in `milox`, not here.

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
