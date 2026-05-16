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
