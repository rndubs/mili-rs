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
use crate::{
    Database, MaterialArg, MeshId, MiliError, QueryArgs, QueryResult, Result, StateValues,
};

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

    /// Derived `centroid` query (`derived.py.__compute_centroid`):
    /// `[state][label][dims]` of element/node centroids, one row per
    /// requested label. Result labels are the sorted-unique
    /// intersection of the requested labels with the class's labels
    /// (upstream `np.intersect1d`); per-label math reuses the
    /// bit-exact [`Self::centroid`] geometry (the landed `measure`
    /// path). `components` is `[ux,uy,uz][..dims]` and `title` is
    /// "Centroid Position" — upstream copies the `nodpos` primal
    /// layout's components.
    pub fn centroid_query(
        &self,
        mesh: MeshId,
        class: &str,
        req_labels: Option<&[i32]>,
        state_idx: &[usize],
    ) -> Result<QueryResult> {
        if self.superclass_code(mesh, class).is_none() {
            return Err(MiliError::UnknownClass(class.to_owned()));
        }
        let class_labels = self.labels(mesh, class)?.unwrap_or_default();
        // np.intersect1d(req, class_labels): sorted, unique, present.
        // No label filter → every class label (sorted unique).
        let mut labels: Vec<i32> = match req_labels {
            Some(req) => {
                let want: HashSet<i32> = req.iter().copied().collect();
                class_labels
                    .iter()
                    .copied()
                    .filter(|l| want.contains(l))
                    .collect()
            }
            None => class_labels.clone(),
        };
        labels.sort_unstable();
        labels.dedup();

        let mut dims = 3usize;
        let mut values: Vec<f32> = Vec::new();
        // Build [state][label][dims]: centroid() returns per-state
        // dims-vectors for one label, so transpose into state-major.
        let mut per_label: Vec<Vec<Vec<f32>>> = Vec::with_capacity(labels.len());
        for &lab in &labels {
            let c = self.centroid(mesh, class, lab, state_idx)?;
            if let Some(first) = c.first() {
                dims = first.len();
            }
            per_label.push(c);
        }
        values.reserve(state_idx.len() * labels.len() * dims);
        for s in 0..state_idx.len() {
            for lc in &per_label {
                values.extend_from_slice(&lc[s]);
            }
        }
        let components: Vec<String> = ["ux", "uy", "uz"][..dims.min(3)]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        Ok(QueryResult {
            values: StateValues::F32(values),
            labels,
            components,
            title: "Centroid Position".to_owned(),
            class_name: class.to_owned(),
        })
    }

    /// The connectivity-coupled gather upstream's geometry derived use
    /// (`__query_required_primals` for a `node` primal class +
    /// `__generate_elem_node_map`): the element labels (sorted-unique
    /// intersection with the class), one `nodpos` query over the
    /// elements' unique node set, and the per-element node→row map.
    ///
    /// Returns `(nodpos, dims, n_node_rows, k, elem_node_map, elems)`
    /// where `nodpos` is flat `[state][node_row][dims]`,
    /// `elem_node_map` is flat `[elem][k]` of row indices into the
    /// `nodpos` node axis, and `elems` the result element labels.
    #[allow(clippy::type_complexity)]
    fn elem_node_gather(
        &self,
        mesh: MeshId,
        class: &str,
        req_labels: Option<&[i32]>,
        state_idx: &[usize],
    ) -> Result<(Vec<f32>, usize, usize, usize, Vec<usize>, Vec<i32>)> {
        let req_owned: Vec<i32> = match req_labels {
            Some(r) => r.to_vec(),
            None => self.labels(mesh, class)?.unwrap_or_default(),
        };
        let (conn, k, elems) = match self.nodes_of_elems(mesh, class, &req_owned)? {
            NodesOfElems::Ok {
                nodes,
                ncols,
                elems,
            } => (nodes, ncols, elems),
            NodesOfElems::UnknownClass => return Err(MiliError::UnknownClass(class.to_owned())),
            _ => {
                return Err(MiliError::Unsupported(
                    "derived geometry: class has no usable connectivity",
                ))
            }
        };
        // Unique node labels (any order; the row map handles alignment).
        let mut uniq: Vec<i32> = conn.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let (data, ret, dims) = self.query_nodpos(mesh, &uniq, state_idx)?;
        let mut row_of: HashMap<i32, usize> = HashMap::with_capacity(ret.len());
        for (i, &l) in ret.iter().enumerate() {
            row_of.entry(l).or_insert(i);
        }
        let mut elem_node_map: Vec<usize> = Vec::with_capacity(conn.len());
        for &nl in &conn {
            elem_node_map.push(*row_of.get(&nl).ok_or(MiliError::Unsupported(
                "derived geometry: element node missing from nodpos gather",
            ))?);
        }
        let n_rows = ret.len();
        Ok((data, dims, n_rows, k, elem_node_map, elems))
    }

    /// Derived `element_volume` (`derived.py.__compute_element_volume`)
    /// — the M_HEX 12-term Griz formula or the M_TET
    /// `w · (u × v) / 6`. Result `[state][elem][1]`, f32.
    pub fn element_volume_query(
        &self,
        mesh: MeshId,
        class: &str,
        req_labels: Option<&[i32]>,
        state_idx: &[usize],
    ) -> Result<QueryResult> {
        let code = self
            .superclass_code(mesh, class)
            .ok_or_else(|| MiliError::UnknownClass(class.to_owned()))?;
        let sclass = Superclass::from_code(i64::from(code)).ok_or(
            MiliError::MalformedDirectory("element_volume: bad superclass"),
        )?;
        let (np, dims, nr, k, em, elems) =
            self.elem_node_gather(mesh, class, req_labels, state_idx)?;
        let ne = elems.len();
        // nodpos accessor: state s, element e, local node j, comp c.
        let at = |s: usize, e: usize, j: usize, c: usize| -> f32 {
            np[s * nr * dims + em[e * k + j] * dims + c]
        };
        let mut out: Vec<f32> = Vec::with_capacity(state_idx.len() * ne);
        for s in 0..state_idx.len() {
            for e in 0..ne {
                let v = if sclass == Superclass::Hex {
                    hex_volume(&|j, c| at(s, e, j, c))
                } else if sclass == Superclass::Tet {
                    tet_volume(&|j, c| at(s, e, j, c))
                } else {
                    return Err(MiliError::Unsupported(
                        "element_volume is only defined for M_HEX / M_TET",
                    ));
                };
                out.push(v);
            }
        }
        Ok(QueryResult {
            values: StateValues::F32(out),
            labels: elems,
            components: vec!["element_volume".to_owned()],
            title: "Element Volume".to_owned(),
            class_name: class.to_owned(),
        })
    }

    /// Derived `area` for M_QUAD (`derived.py.__compute_quad_area`):
    /// `sqrt((e·g - f·f)/16)` of the surface metric. `[state][elem][1]`.
    pub fn quad_area_query(
        &self,
        mesh: MeshId,
        class: &str,
        req_labels: Option<&[i32]>,
        state_idx: &[usize],
    ) -> Result<QueryResult> {
        let (np, dims, nr, k, em, elems) =
            self.elem_node_gather(mesh, class, req_labels, state_idx)?;
        let ne = elems.len();
        let at = |s: usize, e: usize, j: usize, c: usize| -> f32 {
            np[s * nr * dims + em[e * k + j] * dims + c]
        };
        let mut out: Vec<f32> = Vec::with_capacity(state_idx.len() * ne);
        for s in 0..state_idx.len() {
            for e in 0..ne {
                let n = |j: usize, c: usize| at(s, e, j, c);
                let fs1 = -n(0, 0) + n(1, 0) + n(2, 0) - n(3, 0);
                let fs2 = -n(0, 1) + n(1, 1) + n(2, 1) - n(3, 1);
                let fs3 = -n(0, 2) + n(1, 2) + n(2, 2) - n(3, 2);
                let ft1 = -n(0, 0) - n(1, 0) + n(2, 0) + n(3, 0);
                let ft2 = -n(0, 1) - n(1, 1) + n(2, 1) + n(3, 1);
                let ft3 = -n(0, 2) - n(1, 2) + n(2, 2) + n(3, 2);
                let e_ = fs1 * fs1 + fs2 * fs2 + fs3 * fs3;
                let f_ = fs1 * ft1 + fs2 * ft2 + fs3 * ft3;
                let g_ = ft1 * ft1 + ft2 * ft2 + ft3 * ft3;
                let sixteen: f32 = 16.0;
                out.push(((e_ * g_ - f_ * f_) / sixteen).sqrt());
            }
        }
        Ok(QueryResult {
            values: StateValues::F32(out),
            labels: elems,
            components: vec!["area".to_owned()],
            title: "Quad Area".to_owned(),
            class_name: class.to_owned(),
        })
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

/// M_HEX element volume — the verbatim 12-term Griz expansion
/// (`derived.py.__compute_element_volume`, M_HEX branch ~1818-1902).
/// `n(j, c)` is the element's local-node `j` coordinate `c`. Every
/// sub-expression and its evaluation order mirrors the numpy source
/// exactly (each is an element-wise op there; scalar here is
/// bit-identical), `/ 12.0` with the weak-scalar f32 divisor.
fn hex_volume(n: &impl Fn(usize, usize) -> f32) -> f32 {
    let a45 = n(3, 2) - n(4, 2);
    let a24 = n(1, 2) - n(3, 2);
    let a52 = n(4, 2) - n(1, 2);
    let a16 = n(0, 2) - n(5, 2);
    let a31 = n(2, 2) - n(0, 2);
    let a63 = n(5, 2) - n(2, 2);
    let a27 = n(1, 2) - n(6, 2);
    let a74 = n(6, 2) - n(3, 2);
    let a38 = n(2, 2) - n(7, 2);
    let a81 = n(7, 2) - n(0, 2);
    let a86 = n(7, 2) - n(5, 2);
    let a57 = n(4, 2) - n(6, 2);
    let a6345 = a63 - a45;
    let a5238 = a52 - a38;
    let a8624 = a86 - a24;
    let a7416 = a74 - a16;
    let a5731 = a57 - a31;
    let a8127 = a81 - a27;
    let px1 = n(1, 1) * a6345 + n(2, 1) * a24 - n(3, 1) * a5238
        + n(4, 1) * a8624
        + n(5, 1) * a52
        + n(7, 1) * a45;
    let px2 = n(2, 1) * a7416 + n(3, 1) * a31 - n(0, 1) * a6345
        + n(5, 1) * a5731
        + n(6, 1) * a63
        + n(4, 1) * a16;
    let px3 = n(3, 1) * a8127 - n(0, 1) * a24 - n(1, 1) * a7416 - n(6, 1) * a8624
        + n(7, 1) * a74
        + n(5, 1) * a27;
    let px4 = n(0, 1) * a5238 - n(1, 1) * a31 - n(2, 1) * a8127 - n(7, 1) * a5731
        + n(4, 1) * a81
        + n(6, 1) * a38;
    let px5 = -n(7, 1) * a7416 + n(6, 1) * a86 + n(5, 1) * a8127
        - n(0, 1) * a8624
        - n(3, 1) * a81
        - n(1, 1) * a16;
    let px6 = -n(4, 1) * a8127 + n(7, 1) * a57 + n(6, 1) * a5238
        - n(1, 1) * a5731
        - n(0, 1) * a52
        - n(2, 1) * a27;
    let px7 = -n(5, 1) * a5238 - n(4, 1) * a86 + n(7, 1) * a6345 + n(2, 1) * a8624
        - n(1, 1) * a63
        - n(3, 1) * a38;
    let px8 = -n(6, 1) * a6345 - n(5, 1) * a57 + n(4, 1) * a7416 + n(3, 1) * a5731
        - n(2, 1) * a74
        - n(0, 1) * a45;
    let vol = px1 * n(0, 0)
        + px2 * n(1, 0)
        + px3 * n(2, 0)
        + px4 * n(3, 0)
        + px5 * n(4, 0)
        + px6 * n(5, 0)
        + px7 * n(6, 0)
        + px8 * n(7, 0);
    let twelve: f32 = 12.0;
    vol / twelve
}

/// M_TET element volume `w · (u × v) / 6`
/// (`derived.py.__compute_element_volume`, M_TET branch ~1904-1911):
/// `u = n1-n0`, `v = n2-n0`, `w = n3-n0`, `np.cross(u,v)` then the
/// `np.sum(..., axis=2)` left-fold over x,y,z, `/ 6.0` weak-scalar.
fn tet_volume(n: &impl Fn(usize, usize) -> f32) -> f32 {
    let u = [n(1, 0) - n(0, 0), n(1, 1) - n(0, 1), n(1, 2) - n(0, 2)];
    let v = [n(2, 0) - n(0, 0), n(2, 1) - n(0, 1), n(2, 2) - n(0, 2)];
    let w = [n(3, 0) - n(0, 0), n(3, 1) - n(0, 1), n(3, 2) - n(0, 2)];
    let cx = u[1] * v[2] - u[2] * v[1];
    let cy = u[2] * v[0] - u[0] * v[2];
    let cz = u[0] * v[1] - u[1] * v[0];
    let vol = w[0] * cx + w[1] * cy + w[2] * cz;
    let six: f32 = 6.0;
    vol / six
}
