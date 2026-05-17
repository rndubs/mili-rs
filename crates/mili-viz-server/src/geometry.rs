//! Phase 4 M2 mesh prep: turn a `mili-rs` `Database` into the
//! per-state triangulated hull the `GeometryRef` contract delivers.
//!
//! This is the `mili-viz-server` analogue of griz's
//! `MO_class_data.data_buffer` (`reference/griz/Src/mesh.h:208`): a
//! cache of state-invariant topology (triangle index buffer +
//! per-triangle material) plus a per-state vertex buffer pulled from
//! the parity-exact primal `nodpos` query. The encoded blob layout is
//! frozen by `planning/mili-viz/phase-4-m2.md` Decision 11.

use std::collections::HashMap;

use mili_rs::{Database, MeshId, QueryArgs, StateValues, Superclass};

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
        Superclass::Tet | Superclass::Tet10 => {
            &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]]
        }
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
            let Some((rows, ncols)) = db.connectivity_ids(mesh_id, &name).ok().flatten()
            else {
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
            let labels = db
                .labels(mesh_id, &name)
                .ok()
                .flatten()
                .unwrap_or_default();
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
    /// data range `(min, max)` (phase-4-m3.md Decisions 13–15).
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
            // Component 0 of each object (phase-4-m3.md Decision 14).
            Some(
                labels
                    .iter()
                    .enumerate()
                    .map(|(i, &lab)| (lab, v[i * comps]))
                    .collect(),
            )
        };

        let mut scalar = vec![f32::NAN; self.node_count];

        if classes.iter().any(|c| c == "node") {
            // Nodal field: map node label → vertex directly.
            if self.node_labels.len() != self.node_count {
                return None;
            }
            let by_label = query("node")?;
            for (i, &lab) in self.node_labels.iter().enumerate() {
                if let Some(&val) = by_label.get(&lab) {
                    scalar[i] = val as f32;
                }
            }
        } else {
            // Element field: nodal-average onto vertices.
            let mut sum = vec![0.0f64; self.node_count];
            let mut cnt = vec![0u32; self.node_count];
            let mut any = false;
            for ec in &self.elem_classes {
                if !classes.iter().any(|c| c == &ec.name) {
                    continue;
                }
                let Some(by_label) = query(&ec.name) else {
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
        }

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

    /// Encode the current-state hull. `scalar` (per-vertex, phase-4-m3
    /// Decision 14) yields the `MVG2` layout; `None` is the M2 `MVG1`
    /// bare hull, byte-identical to before.
    pub fn encode(&self, db: &Database, state: u32, scalar: Option<&[f32]>) -> Vec<u8> {
        let verts = self.coords_at_state(db, state);
        let n_verts = (verts.len() / 3) as u64;
        let n_idx = self.indices.len() as u64;
        let with_scalar =
            scalar.is_some_and(|s| s.len() == (n_verts as usize) && n_verts > 0);

        let mut buf = Vec::with_capacity(
            24 + verts.len() * 4
                + self.indices.len() * 4
                + self.tri_material.len() * 4
                + if with_scalar { verts.len() / 3 * 4 } else { 0 },
        );
        buf.extend_from_slice(if with_scalar { b"MVG2" } else { b"MVG1" });
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&n_verts.to_le_bytes());
        buf.extend_from_slice(&n_idx.to_le_bytes());
        for f in &verts {
            buf.extend_from_slice(&f.to_le_bytes());
        }
        for i in &self.indices {
            buf.extend_from_slice(&i.to_le_bytes());
        }
        for m in &self.tri_material {
            buf.extend_from_slice(&m.to_le_bytes());
        }
        if with_scalar {
            for v in scalar.unwrap() {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        buf
    }

    pub fn num_vertices(&self) -> u64 {
        self.node_count as u64
    }

    pub fn num_indices(&self) -> u64 {
        self.indices.len() as u64
    }

    pub fn mesh_id(&self) -> MeshId {
        self.mesh_id
    }
}
