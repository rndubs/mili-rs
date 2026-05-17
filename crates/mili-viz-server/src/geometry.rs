//! Phase 4 M2 mesh prep: turn a `mili-rs` `Database` into the
//! per-state triangulated hull the `GeometryRef` contract delivers.
//!
//! This is the `mili-viz-server` analogue of griz's
//! `MO_class_data.data_buffer` (`reference/griz/Src/mesh.h:208`): a
//! cache of state-invariant topology (triangle index buffer +
//! per-triangle material) plus a per-state vertex buffer pulled from
//! the parity-exact primal `nodpos` query. The encoded blob layout is
//! frozen by `planning/mili-viz/phase-4-m2.md` Decision 11.

use std::collections::{BTreeMap, HashMap};

use mili_rs::{Database, MeshId, QueryArgs, QueryResult, StateValues, Superclass};

/// Zero-based local node indices of each hex face, transcribed from
/// `reference/mili-python/src/mili/miliinternal.py:675-682` — the
/// **same** table `mili_rs::Database::surface_strain_query` indexes
/// internally with its `face` argument, so a `face` number and the
/// nodes `scatter_hex_faces` scatters that face's value onto
/// correspond (phase-4-m5c.md Decision 30). This is a connectivity
/// constant, the same category as `triangulation()` (griz `faces.c`);
/// the surface-strain math stays solely in the parity-exact kernel.
const HEX_FACE_NODES: [[usize; 4]; 6] = [
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [0, 4, 7, 3],
    [1, 5, 4, 0],
    [4, 5, 6, 7],
    [0, 3, 2, 1],
];

/// Bare-hull `GeometryRef.layout` (phase-4-m2.md Decision 11) — no
/// scalar field. Unchanged from M2.
pub const LAYOUT: &str = "MVG1:verts_f32x3+idx_u32+trimat_u32";

/// Scalar-hull `GeometryRef.layout` (phase-4-m3.md Decision 14): the
/// M2 blob plus a trailing per-vertex `f32` scalar array.
pub const LAYOUT_SCALAR: &str = "MVG2:verts_f32x3+idx_u32+trimat_u32+scalar_f32";

/// One prepped element class kept for M3 nodal scatter: the element →
/// corner-node-id rows and the parallel element label list (same row
/// order as `connectivity_ids` / `labels`).
struct ElemClass {
    name: String,
    n_nodes: usize,
    /// Flat `[elem * n_nodes]` 0-based node ids.
    conns: Vec<u32>,
    /// Element label per row (`conns.len() / n_nodes` entries).
    labels: Vec<i32>,
}

/// State-invariant mesh topology, built once per `load`.
pub struct MeshTopology {
    mesh_id: MeshId,
    /// Number of node coordinate triples (fortran-id order).
    node_count: usize,
    /// Label at each fortran node index — the key that remaps a
    /// `nodpos` query (returned in label order) back into the
    /// node-array order the connectivity indexes against.
    node_labels: Vec<i32>,
    /// Reference (undeformed) coords, flat `[node*3]`, z padded for 2-D.
    ref_coords: Vec<f32>,
    /// Triangle list into the node array.
    indices: Vec<u32>,
    /// Material id per triangle (`indices.len() / 3` entries).
    tri_material: Vec<u32>,
    /// Per element class, kept for M3 nodal scatter.
    elem_classes: Vec<ElemClass>,
}

/// Per-`Superclass` corner-node triangulation (phase-4-m2.md
/// Decision 11). Indices are into an element's local connectivity;
/// mid-side nodes (e.g. `Tet10`'s 4..9) are intentionally unused — the
/// display hull is the corner hull, as griz extracts in
/// `reference/griz/Src/faces.c`.
fn triangulation(sc: Superclass) -> &'static [[usize; 3]] {
    match sc {
        Superclass::Tri => &[[0, 1, 2]],
        Superclass::Quad => &[[0, 1, 2], [0, 2, 3]],
        Superclass::Tet | Superclass::Tet10 => &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
        Superclass::Pyramid => &[
            [0, 1, 2],
            [0, 2, 3],
            [0, 1, 4],
            [1, 2, 4],
            [2, 3, 4],
            [3, 0, 4],
        ],
        Superclass::Wedge => &[
            [0, 1, 2],
            [3, 5, 4],
            [0, 3, 4],
            [0, 4, 1],
            [1, 4, 5],
            [1, 5, 2],
            [2, 5, 3],
            [2, 3, 0],
        ],
        Superclass::Hex => &[
            [0, 1, 2],
            [0, 2, 3],
            [4, 7, 6],
            [4, 6, 5],
            [0, 4, 5],
            [0, 5, 1],
            [1, 5, 6],
            [1, 6, 2],
            [2, 6, 7],
            [2, 7, 3],
            [3, 7, 4],
            [3, 4, 0],
        ],
        // Node / Truss / Beam / Particle / Inode / Unit / Mat / Mesh /
        // Surface contribute no triangles at M2 (line/point primitives
        // are a Phase-5 renderer concern).
        _ => &[],
    }
}

/// Component-0 `label → value` map from a flat `[label][atom]` query
/// result (phase-4-m3.md Decision 14 — vectors color by component 0).
/// `None` on an empty/degenerate result so `show` falls back to the
/// bare hull (M3 Decision 13).
fn component0_map(vals: StateValues, labels: &[i32]) -> Option<HashMap<i32, f64>> {
    if labels.is_empty() {
        return None;
    }
    let v: Vec<f64> = match vals {
        StateValues::F32(v) => v.into_iter().map(f64::from).collect(),
        StateValues::F64(v) => v,
        StateValues::I32(v) => v.into_iter().map(f64::from).collect(),
        StateValues::I64(v) => v.into_iter().map(|x| x as f64).collect(),
    };
    let comps = v.len() / labels.len();
    if comps == 0 {
        return None;
    }
    Some(
        labels
            .iter()
            .enumerate()
            .map(|(i, &lab)| (lab, v[i * comps]))
            .collect(),
    )
}

impl MeshTopology {
    /// Build the topology cache from the first mesh of an open
    /// database. Returns `None` if the database declares no mesh or no
    /// nodes (nothing drawable).
    pub fn build(db: &Database) -> Option<MeshTopology> {
        let mesh_id = db.meshes().meshes().next()?.id;

        let (flat, dims) = db.node_coords(mesh_id).ok()??;
        if dims == 0 {
            return None;
        }
        let node_count = flat.len() / dims;
        let mut ref_coords = vec![0.0f32; node_count * 3];
        for i in 0..node_count {
            for d in 0..dims.min(3) {
                ref_coords[i * 3 + d] = flat[i * dims + d];
            }
        }

        let node_labels = db
            .labels(mesh_id, "node")
            .ok()
            .flatten()
            .unwrap_or_default();

        let mut indices: Vec<u32> = Vec::new();
        let mut tri_material: Vec<u32> = Vec::new();
        let mut elem_classes: Vec<ElemClass> = Vec::new();

        // Collect class metadata first (immutable borrow of the mesh
        // table) so the connectivity decode below is a fresh borrow.
        let classes: Vec<(String, Superclass)> = db
            .meshes()
            .meshes()
            .find(|m| m.id == mesh_id)
            .map(|m| {
                m.classes()
                    .map(|c| (c.short_name.clone(), c.superclass))
                    .collect()
            })
            .unwrap_or_default();

        for (name, sc) in classes {
            let tris = triangulation(sc);
            if tris.is_empty() {
                continue;
            }
            let Some((rows, ncols)) = db.connectivity_ids(mesh_id, &name).ok().flatten() else {
                continue;
            };
            if ncols < 2 {
                continue;
            }
            let n_nodes = ncols - 1; // last column is the material id
            let max_local = tris.iter().flatten().copied().max().unwrap_or(0);
            if n_nodes <= max_local {
                continue; // connectivity too short for this scheme
            }
            let mut conns: Vec<u32> = Vec::with_capacity((rows.len() / ncols) * n_nodes);
            for row in rows.chunks_exact(ncols) {
                let material = row[n_nodes].max(0) as u32;
                for &nid in &row[..n_nodes] {
                    conns.push(nid.max(0) as u32);
                }
                for tri in tris {
                    let mut ok = true;
                    let mut v = [0u32; 3];
                    for (k, &li) in tri.iter().enumerate() {
                        let nid = row[li];
                        if nid < 0 || (nid as usize) >= node_count {
                            ok = false;
                            break;
                        }
                        v[k] = nid as u32;
                    }
                    if ok {
                        indices.extend_from_slice(&v);
                        tri_material.push(material);
                    }
                }
            }
            let labels = db.labels(mesh_id, &name).ok().flatten().unwrap_or_default();
            // Only keep the class for scatter if the label list lines
            // up with the connectivity rows (the query→element join
            // key); otherwise it still contributes triangles, just no
            // scalar.
            if labels.len() == conns.len() / n_nodes {
                elem_classes.push(ElemClass {
                    name,
                    n_nodes,
                    conns,
                    labels,
                });
            }
        }

        Some(MeshTopology {
            mesh_id,
            node_count,
            node_labels,
            ref_coords,
            indices,
            tri_material,
            elem_classes,
        })
    }

    /// Node coordinates at 1-based `state`, flat `[node*3]`. Uses the
    /// primal `nodpos` query (parity-exact), remapped from query-label
    /// order into node-array order; falls back to the reference coords
    /// if the corpus has no usable `nodpos` (phase-4-m2.md Dec. 12).
    fn coords_at_state(&self, db: &Database, state: u32) -> Vec<f32> {
        let n = db.state_count();
        if state == 0 || n == 0 || self.node_labels.len() != self.node_count {
            return self.ref_coords.clone();
        }
        let state_idx = (state as usize - 1).min(n - 1);
        let args = QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: None,
            states: &[state_idx],
            materials: None,
            ips: None,
            subrec: None,
        };
        let Ok((vals, ret_labels)) = db.query_with_labels(&args) else {
            return self.ref_coords.clone();
        };
        let xyz: Vec<f64> = match vals {
            StateValues::F32(v) => v.into_iter().map(f64::from).collect(),
            StateValues::F64(v) => v,
            _ => return self.ref_coords.clone(),
        };
        if ret_labels.is_empty() {
            return self.ref_coords.clone();
        }
        let dims = xyz.len() / ret_labels.len();
        if dims == 0 {
            return self.ref_coords.clone();
        }
        // label -> row offset in the query result.
        let mut by_label = std::collections::HashMap::with_capacity(ret_labels.len());
        for (row, &lab) in ret_labels.iter().enumerate() {
            by_label.insert(lab, row);
        }
        let mut out = self.ref_coords.clone();
        for (i, &lab) in self.node_labels.iter().enumerate() {
            if let Some(&row) = by_label.get(&lab) {
                for d in 0..dims.min(3) {
                    out[i * 3 + d] = xyz[row * dims + d] as f32;
                }
            }
        }
        out
    }

    /// Per-vertex scalar for `svar` at 1-based `state`, plus the finite
    /// data range `(min, max)` (phase-4-m3.md Decisions 13–15;
    /// phase-4-m5.md Decisions 19–20 add scalar stress invariants;
    /// phase-4-m5b.md Decisions 22–23 add the eigenvalue families).
    /// `None` when the svar resolves to no prepped class or the query
    /// fails — the caller then falls back to the M2 bare hull.
    ///
    /// Element results are nodal-averaged (mean of incident elements,
    /// griz smooth shading); nodal results map node→vertex directly.
    /// Untouched vertices are `f32::NAN`. A multi-component svar
    /// (vector) colors by component 0.
    pub fn vertex_scalar(
        &self,
        db: &Database,
        svar: &str,
        state: u32,
    ) -> Option<(Vec<f32>, f64, f64)> {
        let n = db.state_count();
        if svar.is_empty() || n == 0 || state == 0 {
            return None;
        }
        let state_idx = (state as usize - 1).min(n - 1);

        // Derived: scalar stress invariants. `show pressure` etc. is
        // not a primal svar, so it must be resolved before the primal
        // `classes_of_state_variable` lookup (phase-4-m5.md Decisions
        // 19–20 — reuse the parity-exact `mili-rs` kernel; element-only
        // by construction, fed through M3's nodal scatter).
        if let Some((inv, title)) = mili_rs::stress_invariant_spec(svar) {
            let primal_names = mili_rs::stress_invariant_primals(inv);
            let classes = db.classes_of_state_variable(primal_names[0])?;
            if classes.is_empty() {
                return None;
            }
            return self.scatter_elements(&classes, |class| {
                let mut primals = Vec::with_capacity(primal_names.len());
                for pn in primal_names {
                    let args = QueryArgs {
                        svar: pn,
                        class,
                        labels: None,
                        states: &[state_idx],
                        materials: None,
                        ips: None,
                        subrec: None,
                    };
                    primals.push(db.query_full(&args).ok()?);
                }
                let qr = mili_rs::compute_stress_invariant(inv, &primals, svar, title).ok()?;
                component0_map(qr.values, &qr.labels)
            });
        }

        // Derived: eigenvalue-based stress families (`prin_stress*` /
        // `prin_dev_stress*` / `max_shear_stress`). Same routing seam as
        // the M5 scalar invariants — only the spec/primals/compute calls
        // change (phase-4-m5b.md Decisions 22–23; the eigensolver is
        // already parity-exact in the `mili-rs` core suite).
        if let Some((kind, title)) = mili_rs::principal_stress_spec(svar) {
            let primal_names = mili_rs::principal_stress_primals();
            let classes = db.classes_of_state_variable(primal_names[0])?;
            if classes.is_empty() {
                return None;
            }
            return self.scatter_elements(&classes, |class| {
                let mut primals = Vec::with_capacity(primal_names.len());
                for pn in primal_names {
                    let args = QueryArgs {
                        svar: pn,
                        class,
                        labels: None,
                        states: &[state_idx],
                        materials: None,
                        ips: None,
                        subrec: None,
                    };
                    primals.push(db.query_full(&args).ok()?);
                }
                let qr = mili_rs::compute_principal_stress(kind, &primals, svar, title).ok()?;
                component0_map(qr.values, &qr.labels)
            });
        }

        // Derived: strain invariants (`vol_strain` / `prin_strain*` /
        // `prin_dev_strain*`). `vol_strain` reads only the 3 normal
        // strains; the principals read all 6 (phase-4-m5b.md
        // Decisions 22–23).
        if let Some((kind, title)) = mili_rs::principal_strain_spec(svar) {
            let primal_names = mili_rs::principal_strain_primals(kind);
            let classes = db.classes_of_state_variable(primal_names[0])?;
            if classes.is_empty() {
                return None;
            }
            return self.scatter_elements(&classes, |class| {
                let mut primals = Vec::with_capacity(primal_names.len());
                for pn in primal_names {
                    let args = QueryArgs {
                        svar: pn,
                        class,
                        labels: None,
                        states: &[state_idx],
                        materials: None,
                        ips: None,
                        subrec: None,
                    };
                    primals.push(db.query_full(&args).ok()?);
                }
                let qr = mili_rs::compute_principal_strain(kind, &primals, svar, title).ok()?;
                component0_map(qr.values, &qr.labels)
            });
        }

        // Derived: the `*_alt` griz closed-form trig principal-strain
        // variants (`prin_strain[1-3]_alt` / `prin_dev_strain[1-3]_alt`).
        // The IDENTICAL element nodal-average scatter seam as the
        // non-alt strain branch above — only the `*_spec`/`*_primals`/
        // `compute_*` calls differ — routed through the now
        // parity-gated `mili_rs::compute_principal_strain_alt`
        // (planning/mili-viz/phase-4-m5d.md Decisions 32–34; closes
        // phase-4-m5c.md Decision 28). No proto/blob change.
        if let Some((kind, title)) = mili_rs::principal_strain_alt_spec(svar) {
            let primal_names = mili_rs::principal_strain_alt_primals(kind);
            let classes = db.classes_of_state_variable(primal_names[0])?;
            if classes.is_empty() {
                return None;
            }
            return self.scatter_elements(&classes, |class| {
                let mut primals = Vec::with_capacity(primal_names.len());
                for pn in primal_names {
                    let args = QueryArgs {
                        svar: pn,
                        class,
                        labels: None,
                        states: &[state_idx],
                        materials: None,
                        ips: None,
                        subrec: None,
                    };
                    primals.push(db.query_full(&args).ok()?);
                }
                let qr = mili_rs::compute_principal_strain_alt(kind, &primals, svar, title).ok()?;
                component0_map(qr.values, &qr.labels)
            });
        }

        // Derived: nodal time-derived families (displacement /
        // velocity / acceleration). A node-direct gather through the
        // parity-exact `mili-rs` kernels, mirroring the
        // `crates/mili-py` `query()` nodal dispatch for the single
        // current state (phase-4-m5c.md Decisions 28–29). Only
        // `reference_state == 0` (the upstream default; the viz `show`
        // vocabulary has no reference-state arg, and a non-zero value
        // is an upstream-rejected extension, never a silent wrong
        // answer). The derived names are not primals, so this must
        // resolve before the primal `classes_of_state_variable`
        // lookup.
        {
            let disp_comp = mili_rs::node_disp_spec(svar);
            let disp_mag = mili_rs::node_disp_mag_spec(svar);
            let vel = mili_rs::node_vel_spec(svar);
            let acc = mili_rs::node_acc_spec(svar);
            if disp_comp.is_some() || disp_mag.is_some() || vel.is_some() || acc.is_some() {
                let mesh = self.mesh_id;
                let node_query = |sv: &str, st: &[usize]| -> Option<QueryResult> {
                    let args = QueryArgs {
                        svar: sv,
                        class: "node",
                        labels: None,
                        states: st,
                        materials: None,
                        ips: None,
                        subrec: None,
                    };
                    db.query_full(&args).ok()
                };
                let computed: Option<QueryResult> = if disp_comp.is_some() || disp_mag.is_some() {
                    let dirs: Vec<usize> = match (disp_comp, disp_mag) {
                        (Some((d, _)), _) => vec![d],
                        (_, Some((ds, _))) => ds.to_vec(),
                        _ => unreachable!(),
                    };
                    let title = disp_comp
                        .map(|(_, t)| t)
                        .or(disp_mag.map(|(_, t)| t))
                        .unwrap();
                    let (coords, dims) = db.node_coords(mesh).ok().flatten().unwrap_or_default();
                    let node_labels = db.labels(mesh, "node").ok().flatten().unwrap_or_default();
                    let mut primals: Vec<QueryResult> = Vec::with_capacity(dirs.len());
                    let mut refs: Vec<Vec<f32>> = Vec::with_capacity(dirs.len());
                    let mut ok = true;
                    for &d in &dirs {
                        let pname = mili_rs::node_disp_primal(d);
                        let Some(primal) = node_query(pname, &[state_idx]) else {
                            ok = false;
                            break;
                        };
                        let Ok(reference) = mili_rs::nodal_reference_from_coords(
                            &primal.labels,
                            &node_labels,
                            &coords,
                            dims.max(1),
                            d,
                        ) else {
                            ok = false;
                            break;
                        };
                        primals.push(primal);
                        refs.push(reference);
                    }
                    if !ok {
                        None
                    } else if disp_comp.is_some() {
                        mili_rs::compute_node_displacement(
                            primals.pop().unwrap(),
                            &refs.pop().unwrap(),
                            svar,
                            title,
                        )
                        .ok()
                    } else {
                        mili_rs::compute_node_displacement_magnitude(&primals, &refs, svar, title)
                            .ok()
                    }
                } else {
                    let (dir, title, is_acc) = vel
                        .map(|(d, t)| (d, t, false))
                        .or(acc.map(|(d, t)| (d, t, true)))
                        .unwrap();
                    let pname = mili_rs::node_disp_primal(dir);
                    let max_state = n as i64;
                    let s = state_idx as i64 + 1;
                    let mut needed: Vec<i64> = Vec::new();
                    if is_acc {
                        if s == 1 {
                            needed.extend([1, 2, 3]);
                        } else if s == max_state {
                            needed.extend([max_state, max_state - 1, max_state - 2]);
                        } else {
                            needed.extend([s - 1, s, s + 1]);
                        }
                    } else {
                        needed.push(s);
                        if s != 1 {
                            needed.push(s - 1);
                        }
                    }
                    needed.retain(|&v| v >= 1 && v <= max_state);
                    needed.sort_unstable();
                    needed.dedup();
                    let needed_idx: Vec<usize> = needed.iter().map(|&v| (v - 1) as usize).collect();
                    match node_query(pname, &needed_idx) {
                        None => None,
                        Some(gathered) => {
                            let times = db.times();
                            if is_acc {
                                mili_rs::compute_node_acceleration(
                                    gathered,
                                    &needed,
                                    &[s],
                                    &times,
                                    max_state,
                                    svar,
                                    title,
                                )
                                .ok()
                            } else {
                                mili_rs::compute_node_velocity(
                                    gathered,
                                    &needed,
                                    &[s],
                                    &times,
                                    svar,
                                    title,
                                )
                                .ok()
                            }
                        }
                    }
                };
                let qr = computed?;
                let by_label = component0_map(qr.values, &qr.labels)?;
                return self.node_direct(&by_label);
            }
        }

        // Derived: per-face Hex surface strain (`surfstrain*`). A
        // separate per-face connectivity gather over the parity-exact
        // `Database::surface_strain_query`, kept distinct from the
        // M5/M5b element-class scatter (phase-4-m5c.md Decision 30).
        if let Some((title, jr, ic)) = mili_rs::surfstrain_spec(svar) {
            return self.scatter_hex_faces(db, svar, title, jr, ic, state_idx);
        }

        let classes = db.classes_of_state_variable(svar)?;
        if classes.is_empty() {
            return None;
        }

        let query = |class: &str| -> Option<HashMap<i32, f64>> {
            let args = QueryArgs {
                svar,
                class,
                labels: None,
                states: &[state_idx],
                materials: None,
                ips: None,
                subrec: None,
            };
            let (vals, labels) = db.query_with_labels(&args).ok()?;
            component0_map(vals, &labels)
        };

        if classes.iter().any(|c| c == "node") {
            // Nodal field: map node label → vertex directly.
            let by_label = query("node")?;
            self.node_direct(&by_label)
        } else {
            self.scatter_elements(&classes, query)
        }
    }

    /// Map a node-label-keyed value map onto the mesh vertices
    /// directly (no averaging — one node, one vertex; griz nodal
    /// shading). Extracted verbatim from M3's inline nodal branch so
    /// the primal nodal path stays byte-identical while the M5c
    /// nodal-time families (phase-4-m5c.md Decision 29) reuse it.
    /// Untouched vertices stay `f32::NAN`.
    fn node_direct(&self, by_label: &HashMap<i32, f64>) -> Option<(Vec<f32>, f64, f64)> {
        if self.node_labels.len() != self.node_count {
            return None;
        }
        let mut scalar = vec![f32::NAN; self.node_count];
        for (i, &lab) in self.node_labels.iter().enumerate() {
            if let Some(&val) = by_label.get(&lab) {
                scalar[i] = val as f32;
            }
        }
        Self::finite_range(scalar)
    }

    /// Per-face Hex surface strain scattered onto the mesh
    /// (phase-4-m5c.md Decision 30). For each retained Hex element
    /// class, the parity-exact `Database::surface_strain_query` is
    /// evaluated for each face `1..=6`; that face's per-element value
    /// is nodal-averaged onto the face's 4 corner nodes via
    /// `HEX_FACE_NODES` (the same table the kernel indexes with
    /// `face`). A separate gather from `scatter_elements` — the
    /// M5/M5b element seam and the M3 paths are untouched. `None`
    /// when the corpus has no Hex class or every `surface_strain_query`
    /// fails, so the caller falls back to the M3 bare hull.
    fn scatter_hex_faces(
        &self,
        db: &Database,
        result_name: &str,
        title: &str,
        jr: usize,
        ic: usize,
        state_idx: usize,
    ) -> Option<(Vec<f32>, f64, f64)> {
        let mesh = self.mesh_id;
        let mut sum = vec![0.0f64; self.node_count];
        let mut cnt = vec![0u32; self.node_count];
        let mut any = false;
        for ec in &self.elem_classes {
            if ec.n_nodes < 8 {
                continue;
            }
            let is_hex = db
                .superclass_code(mesh, &ec.name)
                .and_then(|c| Superclass::from_code(i64::from(c)))
                == Some(Superclass::Hex);
            if !is_hex {
                continue;
            }
            for face in 1..=6i64 {
                let Ok(qr) = db.surface_strain_query(
                    mesh,
                    &ec.name,
                    None,
                    &[state_idx],
                    face,
                    jr,
                    ic,
                    result_name,
                    title,
                ) else {
                    continue;
                };
                let Some(by_label) = component0_map(qr.values, &qr.labels) else {
                    continue;
                };
                let fnodes = &HEX_FACE_NODES[(face - 1) as usize];
                for (e, &lab) in ec.labels.iter().enumerate() {
                    let Some(&val) = by_label.get(&lab) else {
                        continue;
                    };
                    any = true;
                    for &local in fnodes {
                        let nid = ec.conns[e * ec.n_nodes + local] as usize;
                        if nid < self.node_count {
                            sum[nid] += val;
                            cnt[nid] += 1;
                        }
                    }
                }
            }
        }
        if !any {
            return None;
        }
        let mut scalar = vec![f32::NAN; self.node_count];
        for i in 0..self.node_count {
            if cnt[i] > 0 {
                scalar[i] = (sum[i] / f64::from(cnt[i])) as f32;
            }
        }
        Self::finite_range(scalar)
    }

    /// Nodal-average a per-element `label → value` map onto the mesh
    /// vertices (mean of incident elements, griz smooth shading;
    /// phase-4-m3.md Decision 14). `value_for_class` yields the
    /// per-element values for one element class (the M3 primal closure
    /// or the M5 derived kernel — Decision 20). Untouched vertices stay
    /// `f32::NAN`. The accumulation order is unchanged from M3 so the
    /// primal path is byte-identical.
    fn scatter_elements(
        &self,
        classes: &[String],
        value_for_class: impl Fn(&str) -> Option<HashMap<i32, f64>>,
    ) -> Option<(Vec<f32>, f64, f64)> {
        let mut scalar = vec![f32::NAN; self.node_count];
        let mut sum = vec![0.0f64; self.node_count];
        let mut cnt = vec![0u32; self.node_count];
        let mut any = false;
        for ec in &self.elem_classes {
            if !classes.iter().any(|c| c == &ec.name) {
                continue;
            }
            let Some(by_label) = value_for_class(&ec.name) else {
                continue;
            };
            any = true;
            for (e, &lab) in ec.labels.iter().enumerate() {
                let Some(&val) = by_label.get(&lab) else {
                    continue;
                };
                for &nid in &ec.conns[e * ec.n_nodes..(e + 1) * ec.n_nodes] {
                    let nid = nid as usize;
                    if nid < self.node_count {
                        sum[nid] += val;
                        cnt[nid] += 1;
                    }
                }
            }
        }
        if !any {
            return None;
        }
        for i in 0..self.node_count {
            if cnt[i] > 0 {
                scalar[i] = (sum[i] / f64::from(cnt[i])) as f32;
            }
        }
        Self::finite_range(scalar)
    }

    /// Finite `(min, max)` of a per-vertex scalar (griz autoscale,
    /// phase-4-m3.md Decision 15). `None` when no sample is finite.
    fn finite_range(scalar: Vec<f32>) -> Option<(Vec<f32>, f64, f64)> {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &s in &scalar {
            if s.is_finite() {
                lo = lo.min(f64::from(s));
                hi = hi.max(f64::from(s));
            }
        }
        if !lo.is_finite() {
            return None; // no finite samples
        }
        Some((scalar, lo, hi))
    }

    /// A material is visible unless `materials` maps it to `false`
    /// (phase-4-m4.md Decision 16 — never-named materials stay
    /// visible; `disable` sets `false`).
    fn material_visible(materials: &BTreeMap<u32, bool>, mat: u32) -> bool {
        materials.get(&mat) != Some(&false)
    }

    /// Triangle list filtered to visible-material triangles, in the
    /// original order (phase-4-m4.md Decision 16). With nothing
    /// disabled this is the full list in the M2/M3 order, so the
    /// encoded blob stays byte-identical for the frozen tests.
    fn visible_triangles(&self, materials: &BTreeMap<u32, bool>) -> (Vec<u32>, Vec<u32>) {
        let mut idx = Vec::with_capacity(self.indices.len());
        let mut mat = Vec::with_capacity(self.tri_material.len());
        for (t, &m) in self.tri_material.iter().enumerate() {
            if Self::material_visible(materials, m) {
                idx.extend_from_slice(&self.indices[t * 3..t * 3 + 3]);
                mat.push(m);
            }
        }
        (idx, mat)
    }

    /// Encode the current-state hull, dropping triangles of disabled
    /// materials (phase-4-m4.md Decision 16). `scalar` (per-vertex,
    /// phase-4-m3 Decision 14) yields the `MVG2` layout; `None` is the
    /// M2 `MVG1` bare hull. With no material disabled the bytes are
    /// identical to M2/M3. Returns the blob and the post-filter
    /// `num_indices` for the `GeometryRef`.
    pub fn encode(
        &self,
        db: &Database,
        state: u32,
        scalar: Option<&[f32]>,
        materials: &BTreeMap<u32, bool>,
    ) -> (Vec<u8>, u64) {
        let verts = self.coords_at_state(db, state);
        let n_verts = (verts.len() / 3) as u64;
        let (indices, tri_material) = self.visible_triangles(materials);
        let n_idx = indices.len() as u64;
        let with_scalar = scalar.is_some_and(|s| s.len() == (n_verts as usize) && n_verts > 0);

        let mut buf = Vec::with_capacity(
            24 + verts.len() * 4
                + indices.len() * 4
                + tri_material.len() * 4
                + if with_scalar { verts.len() / 3 * 4 } else { 0 },
        );
        buf.extend_from_slice(if with_scalar { b"MVG2" } else { b"MVG1" });
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&n_verts.to_le_bytes());
        buf.extend_from_slice(&n_idx.to_le_bytes());
        for f in &verts {
            buf.extend_from_slice(&f.to_le_bytes());
        }
        for i in &indices {
            buf.extend_from_slice(&i.to_le_bytes());
        }
        for m in &tri_material {
            buf.extend_from_slice(&m.to_le_bytes());
        }
        if with_scalar {
            for v in scalar.unwrap() {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        (buf, n_idx)
    }

    pub fn num_vertices(&self) -> u64 {
        self.node_count as u64
    }

    pub fn mesh_id(&self) -> MeshId {
        self.mesh_id
    }
}
