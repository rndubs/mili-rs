//! Phase-H geometry sub-slice: pure mesh/geometry read methods.
//!
//! Bit-exact mirrors of the upstream
//! `reference/mili-python/src/mili/miliinternal.py` geometry methods
//! (`nodes_of_elems` ~920, `nodes_of_material` ~955, `faces` ~649) and
//! the `MiliDatabase.measure` centroid-distance composition
//! (`milidatabase.py:882`). No derived-variable engine, no projection,
//! no reductions — `measure`'s centroid is the self-contained
//! `__compute_centroid` geometry (`derived.py:1962-2008`: NODE → its
//! position, otherwise the mean of the element's first `node_count`
//! node positions, BEAM dropping its 3rd node) over the already
//! parity-correct primal `nodpos` query.
//!
//! Each error-code branch matches upstream's `ReturnCode.ERROR`
//! conditions; the binding maps the typed variants the `MiliDatabase`
//! wrapper raises as `MiliPythonError`.

use std::collections::{HashMap, HashSet};

use crate::mesh::Superclass;
use crate::{Database, MaterialArg, MeshId, MiliError, QueryArgs, Result, StateValues};

/// Result of [`Database::nodes_of_elems`], mirroring upstream's
/// `ReturnCode.ERROR` branches (`miliinternal.py:935-944`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodesOfElems {
    /// `class_sname not in self.__class_to_sclass`.
    UnknownClass,
    /// None of the provided labels exist for the class.
    NoneExist,
    /// `class_sname not in self.__conns_labels`.
    NoConnectivity,
    /// `(node-label connectivity rows, k node columns, element labels)`.
    /// `nodes` is flat row-major `[selected_elem][k]`; `elems` is one
    /// label per selected element. Selection order is upstream's
    /// `np.intersect1d` order (sorted, unique).
    Ok {
        nodes: Vec<i32>,
        ncols: usize,
        elems: Vec<i32>,
    },
}

/// Result of [`Database::faces`], mirroring `miliinternal.py:666-671`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Faces {
    /// `mo_class is None` — the element class does not exist.
    UnknownClass,
    /// `mo_class.sclass != Superclass.M_HEX`.
    NotHex,
    /// `label not in class_labels`.
    LabelMissing,
    /// Faces 1..=6, each four node labels, in upstream's
    /// `face_to_nodes` order (`miliinternal.py:675-682`).
    Ok([[i32; 4]; 6]),
}

// Upstream `face_to_nodes` (`miliinternal.py:675-682`): zero-based
// indices into the hex element's 8-node connectivity row.
const FACE_TO_NODES: [[usize; 4]; 6] = [
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [0, 4, 7, 3],
    [1, 5, 4, 0],
    [4, 5, 6, 7],
    [0, 3, 2, 1],
];

impl Database {
    /// `nodes_of_elems` (`miliinternal.py:920-953`).
    pub fn nodes_of_elems(
        &self,
        mesh: MeshId,
        class: &str,
        elem_labels: &[i32],
    ) -> Result<NodesOfElems> {
        if self.superclass_code(mesh, class).is_none() {
            return Ok(NodesOfElems::UnknownClass);
        }
        let class_labels = self.labels(mesh, class)?.unwrap_or_default();
        let class_set: HashSet<i32> = class_labels.iter().copied().collect();
        if elem_labels.iter().all(|l| !class_set.contains(l)) {
            return Ok(NodesOfElems::NoneExist);
        }
        let Some((conn, ncols)) = self.connectivity_labels(mesh, class)? else {
            return Ok(NodesOfElems::NoConnectivity);
        };
        // np.intersect1d(elem_labels, class_labels): sorted unique
        // values common to both.
        let input_set: HashSet<i32> = elem_labels.iter().copied().collect();
        let mut labels_list: Vec<i32> = class_labels
            .iter()
            .copied()
            .filter(|v| input_set.contains(v))
            .collect();
        labels_list.sort_unstable();
        labels_list.dedup();
        // First positional index of each label in the class label array
        // (upstream's argsort+searchsorted result, used to index the
        // parallel connectivity rows).
        let mut pos: HashMap<i32, usize> = HashMap::with_capacity(class_labels.len());
        for (i, &v) in class_labels.iter().enumerate() {
            pos.entry(v).or_insert(i);
        }
        let k = ncols - 1; // node columns (material column dropped)
        let mut nodes = Vec::with_capacity(labels_list.len() * k);
        let mut elems = Vec::with_capacity(labels_list.len());
        for &v in &labels_list {
            let i = pos[&v];
            nodes.extend_from_slice(&conn[i * ncols..i * ncols + k]);
            elems.push(v);
        }
        Ok(NodesOfElems::Ok {
            nodes,
            ncols: k,
            elems,
        })
    }

    /// `faces` (`miliinternal.py:649-685`). Hex-only.
    pub fn faces(&self, mesh: MeshId, class: &str, label: i32) -> Result<Faces> {
        let Some(code) = self.superclass_code(mesh, class) else {
            return Ok(Faces::UnknownClass);
        };
        if code != Superclass::Hex as i32 {
            return Ok(Faces::NotHex);
        }
        let class_labels = self.labels(mesh, class)?.unwrap_or_default();
        let Some(idx) = class_labels.iter().position(|&l| l == label) else {
            return Ok(Faces::LabelMissing);
        };
        let (conn, ncols) =
            self.connectivity_labels(mesh, class)?
                .ok_or(MiliError::MalformedDirectory(
                    "faces: hex class has no connectivity",
                ))?;
        let k = ncols - 1; // node-label columns (drop material)
        let row = &conn[idx * ncols..idx * ncols + k];
        let mut faces = [[0i32; 4]; 6];
        for (f, map) in FACE_TO_NODES.iter().enumerate() {
            for (j, &n) in map.iter().enumerate() {
                faces[f][j] = row[n];
            }
        }
        Ok(Faces::Ok(faces))
    }

    /// `nodes_of_material` (`miliinternal.py:955-971`): every class of
    /// the material → its element labels → `nodes_of_elems`, then
    /// `np.unique` (sorted, unique) over all node labels.
    pub fn nodes_of_material(&self, mesh: MeshId, mat: &MaterialArg) -> Result<Vec<i32>> {
        let mut acc: Vec<i32> = Vec::new();
        for (cls, lbls) in self.all_labels_of_material(mesh, mat)? {
            if let NodesOfElems::Ok { nodes, .. } = self.nodes_of_elems(mesh, &cls, &lbls)? {
                acc.extend(nodes);
            }
        }
        acc.sort_unstable();
        acc.dedup();
        Ok(acc)
    }

    /// `MiliDatabase.measure` (`milidatabase.py:882-923`): the
    /// Euclidean distance between the centroids of elements A and B at
    /// each requested state. `state_idx` is 0-based. Returns one
    /// distance per state, in `state_idx` order.
    pub fn measure(
        &self,
        mesh: MeshId,
        a_class: &str,
        a_label: i32,
        b_class: &str,
        b_label: i32,
        state_idx: &[usize],
    ) -> Result<Vec<f32>> {
        let ca = self.centroid(mesh, a_class, a_label, state_idx)?;
        let cb = self.centroid(mesh, b_class, b_label, state_idx)?;
        let mut out = Vec::with_capacity(state_idx.len());
        for s in 0..state_idx.len() {
            // Upstream sums (B-A)^2 over x,y,z then sqrt, all float32.
            let mut sum = 0f32;
            for d in 0..ca[s].len() {
                let diff = cb[s][d] - ca[s][d];
                sum += diff * diff;
            }
            out.push(sum.sqrt());
        }
        Ok(out)
    }

    /// Per-state element/node centroid — the self-contained
    /// `__compute_centroid` geometry (`derived.py:1962-2008`) over the
    /// primal `nodpos` query. Returns one `dims`-long coord per state.
    fn centroid(
        &self,
        mesh: MeshId,
        class: &str,
        label: i32,
        state_idx: &[usize],
    ) -> Result<Vec<Vec<f32>>> {
        let code = self
            .superclass_code(mesh, class)
            .ok_or_else(|| MiliError::UnknownClass(class.to_owned()))?;
        let sclass = Superclass::from_code(i64::from(code))
            .ok_or(MiliError::MalformedDirectory("centroid: bad superclass"))?;

        if sclass == Superclass::Node {
            // NODE: centroid is the node position itself.
            let (data, ret, dims) = self.query_nodpos(mesh, &[label], state_idx)?;
            let row = ret
                .iter()
                .position(|&l| l == label)
                .ok_or(MiliError::LabelNotFound {
                    label,
                    class: class.to_owned(),
                })?;
            let nlab = ret.len();
            let mut out = Vec::with_capacity(state_idx.len());
            for s in 0..state_idx.len() {
                let base = s * nlab * dims + row * dims;
                out.push(data[base..base + dims].to_vec());
            }
            return Ok(out);
        }

        // Element: mean of its first `qty_conns` node positions.
        let mut qty = sclass.node_count();
        if sclass == Superclass::Beam {
            qty -= 1; // upstream ignores BEAM's 3rd connectivity node
        }
        if qty == 0 {
            return Err(MiliError::Unsupported(
                "measure: class has no connectivity for a centroid",
            ));
        }
        let class_labels = self.labels(mesh, class)?.unwrap_or_default();
        let idx =
            class_labels
                .iter()
                .position(|&l| l == label)
                .ok_or(MiliError::LabelNotFound {
                    label,
                    class: class.to_owned(),
                })?;
        let (conn, ncols) =
            self.connectivity_labels(mesh, class)?
                .ok_or(MiliError::MalformedDirectory(
                    "measure: element class has no connectivity",
                ))?;
        let node_labels: Vec<i32> = conn[idx * ncols..idx * ncols + qty].to_vec();
        let (data, ret, dims) = self.query_nodpos(mesh, &node_labels, state_idx)?;
        let mut row_of: HashMap<i32, usize> = HashMap::with_capacity(ret.len());
        for (i, &l) in ret.iter().enumerate() {
            row_of.entry(l).or_insert(i);
        }
        let nlab = ret.len();
        // qty is a superclass node count (<= 10); f32 is exact here and
        // upstream divides the float32 sum by this same python int.
        #[allow(clippy::cast_precision_loss)]
        let qty_f = qty as f32;
        let mut out = Vec::with_capacity(state_idx.len());
        for s in 0..state_idx.len() {
            let mut c = vec![0f32; dims];
            for &nl in &node_labels {
                let r = row_of[&nl];
                let base = s * nlab * dims + r * dims;
                for d in 0..dims {
                    c[d] += data[base + d];
                }
            }
            for v in &mut c {
                *v /= qty_f;
            }
            out.push(c);
        }
        Ok(out)
    }

    /// Primal `nodpos` for a node-label set across states. Returns the
    /// flat `[state][label][dim]` f32 buffer, the entity-axis labels in
    /// row order, and `dims` (atoms per node).
    fn query_nodpos(
        &self,
        _mesh: MeshId,
        node_labels: &[i32],
        state_idx: &[usize],
    ) -> Result<(Vec<f32>, Vec<i32>, usize)> {
        let mut seen: HashSet<i32> = HashSet::new();
        let mut filtered: Vec<i32> = Vec::with_capacity(node_labels.len());
        for &l in node_labels {
            if seen.insert(l) {
                filtered.push(l);
            }
        }
        let args = QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: Some(&filtered),
            states: state_idx,
            materials: None,
            ips: None,
            subrec: None,
        };
        let (vals, ret_labels) = self.query_with_labels(&args)?;
        let StateValues::F32(data) = vals else {
            return Err(MiliError::MalformedDirectory(
                "nodpos query returned a non-f32 buffer",
            ));
        };
        let denom = state_idx.len().max(1) * ret_labels.len().max(1);
        let dims = data.len() / denom;
        Ok((data, ret_labels, dims))
    }
}
