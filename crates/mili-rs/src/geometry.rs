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
use std::ops::{Add, Div, Mul, Neg, Sub};

use crate::mesh::Superclass;
use crate::{
    Database, MaterialArg, MeshId, MiliError, QueryArgs, QueryResult, Result, StateValues,
};

/// The float element type a `nodpos`-driven geometry kernel runs in.
/// Upstream computes every geometry derived (`element_volume`, `area`,
/// `centroid`, `measure`) element-wise in the primal `nodpos` dtype —
/// f32 for a single-precision database, f64 for a double-precision one
/// (`dbl_nodtang`) — so the kernels are generic over `T` and the f32
/// instantiation is bit-identical to the original f32-only code (same
/// ops, same exactly-representable integer divisors). `relative_volume`
/// is the one exception: its jacobian math is always `np.float64` and
/// the result always f32, so it reads the gathered coords as f64
/// regardless via the standalone [`nodpos_f64`] helper.
trait GeomF:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    const ZERO: Self;
    /// An exact small integer divisor (`12`, `6`, `16`, a node count);
    /// every value used here is exactly representable in f32 and f64,
    /// so this matches the upstream weak-scalar Python `int` / float
    /// divisor bit-for-bit.
    fn from_count(n: usize) -> Self;
    fn sqrt(self) -> Self;
}

impl GeomF for f32 {
    const ZERO: f32 = 0.0;
    #[allow(clippy::cast_precision_loss)]
    fn from_count(n: usize) -> f32 {
        n as f32
    }
    fn sqrt(self) -> f32 {
        f32::sqrt(self)
    }
}

impl GeomF for f64 {
    const ZERO: f64 = 0.0;
    #[allow(clippy::cast_precision_loss)]
    fn from_count(n: usize) -> f64 {
        n as f64
    }
    fn sqrt(self) -> f64 {
        f64::sqrt(self)
    }
}

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

// Trilinear 8-node shape-function natural derivatives for
// `relative_volume` (`derived.py:2328-2330`).
const DN1: [f64; 8] = [-0.125, -0.125, 0.125, 0.125, -0.125, -0.125, 0.125, 0.125];
const DN2: [f64; 8] = [-0.125, -0.125, -0.125, -0.125, 0.125, 0.125, 0.125, 0.125];
const DN3: [f64; 8] = [-0.125, 0.125, 0.125, -0.125, -0.125, 0.125, 0.125, -0.125];

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
        let (ca, da, ns) = self.centroid(mesh, a_class, a_label, state_idx)?;
        let (cb, db, _) = self.centroid(mesh, b_class, b_label, state_idx)?;
        let dims = da.min(db);
        // Upstream sums (B-A)^2 over x,y,z then sqrt, in the centroid
        // (= nodpos primal) dtype. `ca`/`cb` come from the same
        // database so they share a variant. The f32 path is
        // bit-identical to the original; the f64 path (no measure
        // value-test corpus is double-precision) mirrors the same op
        // order at f64, cast to f32 for the public Vec<f32> contract.
        let out: Vec<f32> = match (&ca, &cb) {
            (StateValues::F64(a), StateValues::F64(b)) => {
                (0..ns).map(|s| measure_one(a, b, s, dims) as f32).collect()
            }
            (StateValues::F32(a), StateValues::F32(b)) => {
                (0..ns).map(|s| measure_one(a, b, s, dims)).collect()
            }
            _ => {
                return Err(MiliError::MalformedDirectory(
                    "measure: centroid is not a float buffer",
                ))
            }
        };
        Ok(out)
    }

    /// Per-state element/node centroid — the self-contained
    /// `__compute_centroid` geometry (`derived.py:1962-2008`) over the
    /// primal `nodpos` query. Returns a flat `[state][dims]` buffer in
    /// the primal `nodpos` dtype (f32 / f64 — `dbl_nodtang` is f64),
    /// the `dims`, and the state count. The gather is dtype-agnostic;
    /// the mean/copy kernel is generic so the f32 path stays
    /// bit-identical and the f64 path mirrors the same op order.
    fn centroid(
        &self,
        mesh: MeshId,
        class: &str,
        label: i32,
        state_idx: &[usize],
    ) -> Result<(StateValues, usize, usize)> {
        let code = self
            .superclass_code(mesh, class)
            .ok_or_else(|| MiliError::UnknownClass(class.to_owned()))?;
        let sclass = Superclass::from_code(i64::from(code))
            .ok_or(MiliError::MalformedDirectory("centroid: bad superclass"))?;
        let ns = state_idx.len();

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
            let pick = |src: &StateValues| -> StateValues {
                match src {
                    StateValues::F32(v) => {
                        StateValues::F32(centroid_node_kernel(v, ns, nlab, dims, row))
                    }
                    StateValues::F64(v) => {
                        StateValues::F64(centroid_node_kernel(v, ns, nlab, dims, row))
                    }
                    _ => StateValues::F32(Vec::new()),
                }
            };
            return Ok((pick(&data), dims, ns));
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
        let rows: Vec<usize> = node_labels.iter().map(|nl| row_of[nl]).collect();
        let values = match &data {
            StateValues::F32(v) => StateValues::F32(centroid_mean_kernel(v, ns, nlab, dims, &rows)),
            StateValues::F64(v) => StateValues::F64(centroid_mean_kernel(v, ns, nlab, dims, &rows)),
            _ => {
                return Err(MiliError::MalformedDirectory(
                    "centroid: nodpos is not a float buffer",
                ))
            }
        };
        Ok((values, dims, ns))
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

        let ns = state_idx.len();
        let mut dims = 3usize;
        // centroid() returns a flat `[state][dims]` buffer per label;
        // transpose into state-major `[state][label][dims]`, preserving
        // the primal `nodpos` dtype (f32 / f64).
        let mut per_label: Vec<StateValues> = Vec::with_capacity(labels.len());
        for &lab in &labels {
            let (c, d, _) = self.centroid(mesh, class, lab, state_idx)?;
            dims = d;
            per_label.push(c);
        }
        let interleave_f32 = |buf: &mut Vec<f32>| {
            for s in 0..ns {
                for lc in &per_label {
                    if let StateValues::F32(v) = lc {
                        buf.extend_from_slice(&v[s * dims..s * dims + dims]);
                    }
                }
            }
        };
        let interleave_f64 = |buf: &mut Vec<f64>| {
            for s in 0..ns {
                for lc in &per_label {
                    if let StateValues::F64(v) = lc {
                        buf.extend_from_slice(&v[s * dims..s * dims + dims]);
                    }
                }
            }
        };
        let cap = ns * labels.len() * dims;
        let values = if let Some(StateValues::F64(_)) = per_label.first() {
            let mut b = Vec::with_capacity(cap);
            interleave_f64(&mut b);
            StateValues::F64(b)
        } else {
            let mut b = Vec::with_capacity(cap);
            interleave_f32(&mut b);
            StateValues::F32(b)
        };
        let components: Vec<String> = ["ux", "uy", "uz"][..dims.min(3)]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        Ok(QueryResult {
            values,
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
    ) -> Result<(
        StateValues,
        usize,
        usize,
        usize,
        Vec<usize>,
        Vec<i32>,
        Vec<i32>,
    )> {
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
        Ok((data, dims, n_rows, k, elem_node_map, elems, ret))
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
        let (np, dims, nr, k, em, elems, _ret) =
            self.elem_node_gather(mesh, class, req_labels, state_idx)?;
        let ne = elems.len();
        let ns = state_idx.len();
        let values = match &np {
            StateValues::F32(d) => {
                StateValues::F32(element_volume_kernel(d, dims, nr, k, &em, ne, ns, sclass)?)
            }
            StateValues::F64(d) => {
                StateValues::F64(element_volume_kernel(d, dims, nr, k, &em, ne, ns, sclass)?)
            }
            _ => {
                return Err(MiliError::MalformedDirectory(
                    "element_volume: nodpos is not a float buffer",
                ))
            }
        };
        Ok(QueryResult {
            values,
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
        let (np, dims, nr, k, em, elems, _ret) =
            self.elem_node_gather(mesh, class, req_labels, state_idx)?;
        let ne = elems.len();
        let ns = state_idx.len();
        let values = match &np {
            StateValues::F32(d) => StateValues::F32(quad_area_kernel(d, dims, nr, k, &em, ne, ns)),
            StateValues::F64(d) => StateValues::F64(quad_area_kernel(d, dims, nr, k, &em, ne, ns)),
            _ => {
                return Err(MiliError::MalformedDirectory(
                    "area: nodpos is not a float buffer",
                ))
            }
        };
        Ok(QueryResult {
            values,
            labels: elems,
            components: vec!["area".to_owned()],
            title: "Quad Area".to_owned(),
            class_name: class.to_owned(),
        })
    }

    /// Derived `relative_volume` (`derived.py.__compute_relative_volume`)
    /// — `det F`, the deformation-gradient determinant of the trilinear
    /// 8-node map: build the reference Jacobian from the initial node
    /// coordinates (`reference_state == 0`), invert it, form the
    /// current-config gradient `F`, take `det F`. M_TET expands its 4
    /// nodes via `[0,1,2,2,3,3,3,3]`. All intermediate math is f64
    /// (upstream's `np.float64` jacob/invJacob/F arrays), result f32.
    #[allow(clippy::too_many_lines)]
    pub fn relative_volume_query(
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
            MiliError::MalformedDirectory("relative_volume: bad superclass"),
        )?;
        // Local-node → 8-node-hex expansion.
        let map: [usize; 8] = match sclass {
            Superclass::Hex => [0, 1, 2, 3, 4, 5, 6, 7],
            Superclass::Tet => [0, 1, 2, 2, 3, 3, 3, 3],
            _ => {
                return Err(MiliError::Unsupported(
                    "relative_volume is only defined for M_HEX / M_TET",
                ))
            }
        };
        let (np, dims, nr, k, em, elems, ret) =
            self.elem_node_gather(mesh, class, req_labels, state_idx)?;

        // Reference (state-0) node coordinates aligned to the gathered
        // node rows: db.nodes()[ordinal-of-label].
        let (ncoords, ndims) = self.node_coords(mesh)?.unwrap_or_default();
        let node_lbls = self.labels(mesh, "node")?.unwrap_or_default();
        let mut ord_of: HashMap<i32, usize> = HashMap::with_capacity(node_lbls.len());
        for (i, &l) in node_lbls.iter().enumerate() {
            ord_of.entry(l).or_insert(i);
        }
        let mut ref_xyz: Vec<[f64; 3]> = Vec::with_capacity(nr);
        for &lab in &ret {
            let o = *ord_of.get(&lab).ok_or(MiliError::Unsupported(
                "relative_volume: gathered node missing from nodes()",
            ))?;
            ref_xyz.push([
                f64::from(ncoords[o * ndims]),
                f64::from(ncoords[o * ndims + 1]),
                f64::from(ncoords[o * ndims + 2]),
            ]);
        }

        let ne = elems.len();
        let ns = state_idx.len();
        let mut out: Vec<f32> = vec![0.0; ns * ne];
        for e in 0..ne {
            // The element's 8 hex-equivalent node rows.
            let idx8: [usize; 8] = std::array::from_fn(|j| em[e * k + map[j]]);
            let rx: [f64; 8] = std::array::from_fn(|j| ref_xyz[idx8[j]][0]);
            let ry: [f64; 8] = std::array::from_fn(|j| ref_xyz[idx8[j]][1]);
            let rz: [f64; 8] = std::array::from_fn(|j| ref_xyz[idx8[j]][2]);
            let dot = |a: &[f64; 8], b: &[f64; 8]| -> f64 {
                let mut s = 0.0;
                for j in 0..8 {
                    s += a[j] * b[j];
                }
                s
            };
            // Reference Jacobian (row-major 3x3 as [0..9]).
            let j0 = dot(&DN1, &rx);
            let j1 = dot(&DN1, &ry);
            let j2 = dot(&DN1, &rz);
            let j3 = dot(&DN2, &rx);
            let j4 = dot(&DN2, &ry);
            let j5 = dot(&DN2, &rz);
            let j6 = dot(&DN3, &rx);
            let j7 = dot(&DN3, &ry);
            let j8 = dot(&DN3, &rz);
            let det = j0 * j4 * j8 + j1 * j5 * j6 + j2 * j3 * j7
                - j2 * j4 * j6
                - j1 * j3 * j8
                - j0 * j5 * j7;
            let dj = 1.0 / det;
            let i0 = dj * (j4 * j8 - j5 * j7);
            let i1 = dj * (-j1 * j8 + j2 * j7);
            let i2 = dj * (j1 * j5 - j2 * j4);
            let i3 = dj * (-j3 * j8 + j5 * j6);
            let i4 = dj * (j0 * j8 - j2 * j6);
            let i5 = dj * (-j0 * j5 + j2 * j3);
            let i6 = dj * (j3 * j7 - j4 * j6);
            let i7 = dj * (-j0 * j7 + j1 * j6);
            let i8 = dj * (j0 * j4 - j1 * j3);
            // Spatial shape-function gradients.
            let dnx: [f64; 8] = std::array::from_fn(|j| i0 * DN1[j] + i1 * DN2[j] + i2 * DN3[j]);
            let dny: [f64; 8] = std::array::from_fn(|j| i3 * DN1[j] + i4 * DN2[j] + i5 * DN3[j]);
            let dnz: [f64; 8] = std::array::from_fn(|j| i6 * DN1[j] + i7 * DN2[j] + i8 * DN3[j]);
            for (s, _) in state_idx.iter().enumerate() {
                let cx: [f64; 8] =
                    std::array::from_fn(|j| nodpos_f64(&np, s * nr * dims + idx8[j] * dims));
                let cy: [f64; 8] =
                    std::array::from_fn(|j| nodpos_f64(&np, s * nr * dims + idx8[j] * dims + 1));
                let cz: [f64; 8] =
                    std::array::from_fn(|j| nodpos_f64(&np, s * nr * dims + idx8[j] * dims + 2));
                let f0 = dot(&dnx, &cx);
                let f1 = dot(&dny, &cx);
                let f2 = dot(&dnz, &cx);
                let f3 = dot(&dnx, &cy);
                let f4 = dot(&dny, &cy);
                let f5 = dot(&dnz, &cy);
                let f6 = dot(&dnx, &cz);
                let f7 = dot(&dny, &cz);
                let f8 = dot(&dnz, &cz);
                let detf = f0 * f4 * f8 + f1 * f5 * f6 + f2 * f3 * f7
                    - f2 * f4 * f6
                    - f1 * f3 * f8
                    - f0 * f5 * f7;
                out[s * ne + e] = detf as f32;
            }
        }
        Ok(QueryResult {
            values: StateValues::F32(out),
            labels: elems,
            components: vec!["relative_volume".to_owned()],
            title: "Relative Volume".to_owned(),
            class_name: class.to_owned(),
        })
    }

    /// Primal `nodpos` for a node-label set across states. Returns the
    /// flat `[state][label][dim]` buffer (f32 *or* f64 — the primal
    /// dtype, double-precision for `dbl_nodtang`), the entity-axis
    /// labels in row order, and `dims` (atoms per node).
    fn query_nodpos(
        &self,
        _mesh: MeshId,
        node_labels: &[i32],
        state_idx: &[usize],
    ) -> Result<(StateValues, Vec<i32>, usize)> {
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
        if !matches!(vals, StateValues::F32(_) | StateValues::F64(_)) {
            return Err(MiliError::MalformedDirectory(
                "nodpos query returned a non-float buffer",
            ));
        }
        let denom = state_idx.len().max(1) * ret_labels.len().max(1);
        let dims = vals.len() / denom;
        Ok((vals, ret_labels, dims))
    }
}

/// M_HEX element volume — the verbatim 12-term Griz expansion
/// (`derived.py.__compute_element_volume`, M_HEX branch ~1818-1902).
/// `n(j, c)` is the element's local-node `j` coordinate `c`. Every
/// sub-expression and its evaluation order mirrors the numpy source
/// exactly (each is an element-wise op there; scalar here is
/// bit-identical), `/ 12.0` with the weak-scalar f32 divisor.
fn hex_volume<T: GeomF>(n: &impl Fn(usize, usize) -> T) -> T {
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
    vol / T::from_count(12)
}

/// M_TET element volume `w · (u × v) / 6`
/// (`derived.py.__compute_element_volume`, M_TET branch ~1904-1911):
/// `u = n1-n0`, `v = n2-n0`, `w = n3-n0`, `np.cross(u,v)` then the
/// `np.sum(..., axis=2)` left-fold over x,y,z, `/ 6.0` weak-scalar.
fn tet_volume<T: GeomF>(n: &impl Fn(usize, usize) -> T) -> T {
    let u = [n(1, 0) - n(0, 0), n(1, 1) - n(0, 1), n(1, 2) - n(0, 2)];
    let v = [n(2, 0) - n(0, 0), n(2, 1) - n(0, 1), n(2, 2) - n(0, 2)];
    let w = [n(3, 0) - n(0, 0), n(3, 1) - n(0, 1), n(3, 2) - n(0, 2)];
    let cx = u[1] * v[2] - u[2] * v[1];
    let cy = u[2] * v[0] - u[0] * v[2];
    let cz = u[0] * v[1] - u[1] * v[0];
    let vol = w[0] * cx + w[1] * cy + w[2] * cz;
    vol / T::from_count(6)
}

/// Read a single flat element of a float `nodpos` buffer widened to
/// f64 — `relative_volume`'s jacobian math is always `np.float64`
/// (`derived.py:2324-2334`), so it consumes the gathered coords as
/// f64 regardless of the on-disk precision. Out-of-range / non-float
/// yields `0.0` (callers index within the gathered extent).
fn nodpos_f64(sv: &StateValues, i: usize) -> f64 {
    match sv {
        StateValues::F32(v) => v.get(i).copied().map_or(0.0, f64::from),
        StateValues::F64(v) => v.get(i).copied().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// NODE centroid: the node's own position. Flat `[state][dims]`.
fn centroid_node_kernel<T: GeomF>(
    data: &[T],
    ns: usize,
    nlab: usize,
    dims: usize,
    row: usize,
) -> Vec<T> {
    let mut out = Vec::with_capacity(ns * dims);
    for s in 0..ns {
        let base = s * nlab * dims + row * dims;
        out.extend_from_slice(&data[base..base + dims]);
    }
    out
}

/// Element centroid: mean of the element's `rows.len()` node
/// positions, divided by the (exact, weak-scalar) node count. Flat
/// `[state][dims]`. `T` is the primal `nodpos` dtype.
fn centroid_mean_kernel<T: GeomF>(
    data: &[T],
    ns: usize,
    nlab: usize,
    dims: usize,
    rows: &[usize],
) -> Vec<T> {
    let qty = T::from_count(rows.len());
    let mut out = Vec::with_capacity(ns * dims);
    for s in 0..ns {
        let mut c = vec![T::ZERO; dims];
        for &r in rows {
            let base = s * nlab * dims + r * dims;
            for d in 0..dims {
                c[d] = c[d] + data[base + d];
            }
        }
        for v in &mut c {
            *v = *v / qty;
        }
        out.extend_from_slice(&c);
    }
    out
}

/// `measure`: `sqrt(sum_d (b - a)^2)` over one state's `dims` centroid
/// components, in the centroid dtype `T`.
fn measure_one<T: GeomF>(a: &[T], b: &[T], s: usize, dims: usize) -> T {
    let mut sum = T::ZERO;
    for d in 0..dims {
        let diff = b[s * dims + d] - a[s * dims + d];
        sum = sum + diff * diff;
    }
    sum.sqrt()
}

/// `element_volume` numeric kernel, generic over the primal `nodpos`
/// dtype `T` (f32 / f64). `np` is the flat `[state][node_row][dims]`
/// gathered buffer; `em` the `[elem][k]` node-row map.
#[allow(clippy::too_many_arguments)]
fn element_volume_kernel<T: GeomF>(
    np: &[T],
    dims: usize,
    nr: usize,
    k: usize,
    em: &[usize],
    ne: usize,
    ns: usize,
    sclass: Superclass,
) -> Result<Vec<T>> {
    let at = |s: usize, e: usize, j: usize, c: usize| -> T {
        np[s * nr * dims + em[e * k + j] * dims + c]
    };
    let mut out: Vec<T> = Vec::with_capacity(ns * ne);
    for s in 0..ns {
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
    Ok(out)
}

/// `area` (M_QUAD) numeric kernel: `sqrt((e·g - f·f)/16)` of the
/// surface metric, generic over the primal `nodpos` dtype `T`.
fn quad_area_kernel<T: GeomF>(
    np: &[T],
    dims: usize,
    nr: usize,
    k: usize,
    em: &[usize],
    ne: usize,
    ns: usize,
) -> Vec<T> {
    let at = |s: usize, e: usize, j: usize, c: usize| -> T {
        np[s * nr * dims + em[e * k + j] * dims + c]
    };
    let mut out: Vec<T> = Vec::with_capacity(ns * ne);
    for s in 0..ns {
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
            out.push(((e_ * g_ - f_ * f_) / T::from_count(16)).sqrt());
        }
    }
    out
}
