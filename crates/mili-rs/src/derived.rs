//! Phase-H derived-variable sub-slice (node displacement).
//!
//! Bit-exact mirror of the upstream node-displacement derived
//! expression (`reference/mili-python/src/mili/derived.py`:
//! `__compute_node_displacement` ~978 + `__get_nodal_reference_positions`
//! ~948, `reference_state=0`): `disp_<dir> = <ux|uy|uz> - <reference>`
//! where the reference at the default `reference_state=0` is the
//! initial nodal coordinate (`db.nodes()`), selected by the ordinals of
//! the primal-returned node labels within the node class's label list.
//!
//! Only the node-displacement family (`disp_x` / `disp_y` / `disp_z`)
//! is ported here — the rest of `derived.py` (stress/strain
//! invariants, velocities, accelerations, the derived-listing methods)
//! stays a later sub-slice. The reduction (`ResultModifier`) math is a
//! decision-18 Python-over-primal post-process and lives in `milox`,
//! not here.

use crate::error::{MiliError, Result};
use crate::query::{QueryResult, StateValues};

/// Resolve a node-displacement derived name to `(component direction,
/// title)`. `disp_x` → `(0, "X Displacement")`, etc. Returns `None`
/// for any name that is not a ported node-displacement derived.
pub fn node_disp_spec(name: &str) -> Option<(usize, &'static str)> {
    match name {
        "disp_x" => Some((0, "X Displacement")),
        "disp_y" => Some((1, "Y Displacement")),
        "disp_z" => Some((2, "Z Displacement")),
        _ => None,
    }
}

/// The primal state variable a node-displacement derived needs:
/// `disp_x` ← `ux`, `disp_y` ← `uy`, `disp_z` ← `uz`
/// (`derived.py:59/67/74`).
pub fn node_disp_primal(dir: usize) -> &'static str {
    ["ux", "uy", "uz"][dir]
}

/// Compute a node-displacement derived result from its primal
/// `ux`/`uy`/`uz` query and the initial nodal coordinates.
///
/// Mirrors `derived.py.__compute_node_displacement` with
/// `reference_state=0`: the reference for primal-returned node label
/// `primal.labels[k]` is `node_coords[ordinals[k] * dims + dir]`, where
/// `ordinals` are the ascending indices into `node_labels` whose label
/// appears in the primal-returned label set (upstream
/// `np.where(np.isin(labels_of_class, labels))[0]`). `disp = primal -
/// reference`, broadcast over states.
pub fn compute_node_displacement(
    primal: QueryResult,
    node_labels: &[i32],
    node_coords: &[f32],
    dims: usize,
    dir: usize,
    result_name: &str,
    title: &str,
) -> Result<QueryResult> {
    let StateValues::F32(prim) = primal.values else {
        return Err(MiliError::Unsupported(
            "node displacement requires float32 nodal position primals",
        ));
    };
    let n_lab = primal.labels.len();

    // ordinals: ascending positions in `node_labels` whose label is in
    // the primal-returned label set (upstream `isin(labels_of_class,
    // labels)`), giving one reference row per primal label row.
    let queried: std::collections::HashSet<i32> = primal.labels.iter().copied().collect();
    let ordinals: Vec<usize> = node_labels
        .iter()
        .enumerate()
        .filter(|(_, l)| queried.contains(l))
        .map(|(i, _)| i)
        .collect();

    if ordinals.len() != n_lab {
        return Err(MiliError::Unsupported(
            "node-displacement reference/primal label count mismatch",
        ));
    }

    let reference: Vec<f32> = ordinals
        .iter()
        .map(|&ord| {
            node_coords
                .get(ord * dims + dir)
                .copied()
                .ok_or(MiliError::Unsupported(
                    "node-displacement reference index out of range",
                ))
        })
        .collect::<Result<_>>()?;

    // `prim` is `[state][label]` row-major; the reference is one value
    // per label, repeated every state — `cycle()` aligns it with each
    // state block. `disp = prim − reference` (upstream broadcast).
    let disp: Vec<f32> = if n_lab == 0 {
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
