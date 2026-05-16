//! Phase-H adjacency + geometric-mesh-info sub-slice.
//!
//! Bit-exact ports of the upstream `GeometricMeshInfo`
//! (`reference/mili-python/src/mili/geometric_mesh_info.py`) and the
//! serial `AdjacencyMapping`
//! (`reference/mili-python/src/mili/adjacency.py`) — pure mesh /
//! coordinate topology built on the already parity-correct
//! connectivity + `nodpos` query surface. No derived-variable engine,
//! no projection, no reductions.
//!
//! `GeometricMeshInfo` is the `_MiliInternal.geometry` object; the
//! `AdjacencyMapping` serial branches (`self.serial == True`,
//! `merge_results == True`) collapse to identity, so the milox
//! adapters just forward to these core methods.

use std::collections::HashSet;

use crate::mesh::Superclass;
use crate::{
    Database, MaterialArg, MeshId, MiliError, NodesOfElems, QueryArgs, Result, StateValues,
};

/// `mesh_entities_*` result: per-class label arrays, in key order,
/// with a trailing `("node", …)` entry.
type ClassLabels = Vec<(String, Vec<i32>)>;

/// Result of [`Database::adj_neighbor_elements`], mirroring upstream's
/// `AdjacencyMapping.neighbor_elements` `ValueError` branches
/// (`adjacency.py:174-180`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeighborElems {
    /// `self.mili.labels(entity_type)` is empty / class unknown.
    NoLabels,
    /// `label not in labels`.
    LabelMissing,
    /// Per-class neighbor element labels, in connectivity-key order.
    Ok(Vec<(String, Vec<i32>)>),
}

/// A one-state `nodpos` buffer in its on-disk precision. Distance
/// math widens to `f64` (numpy promotes `float32 - float64`); centroid
/// sums stay in the array dtype (NEP50).
enum NodposBuf {
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl NodposBuf {
    fn len(&self) -> usize {
        match self {
            Self::F32(d) => d.len(),
            Self::F64(d) => d.len(),
        }
    }

    fn at_f64(&self, i: usize) -> f64 {
        match self {
            Self::F32(d) => f64::from(d[i]),
            Self::F64(d) => d[i],
        }
    }

    /// `np.sum(nodpos[node_ids], axis=0) / len(node_ids)` in the
    /// array dtype, widened to `f64` for the caller's float64 norm.
    fn centroid(&self, ids: &[i32], dims: usize) -> Vec<f64> {
        match self {
            Self::F32(d) => {
                #[allow(clippy::cast_precision_loss)]
                let k = ids.len() as f32;
                (0..dims)
                    .map(|dim| {
                        let mut s = 0f32;
                        for &id in ids {
                            s += d[id as usize * dims + dim];
                        }
                        f64::from(s / k)
                    })
                    .collect()
            }
            Self::F64(d) => {
                #[allow(clippy::cast_precision_loss)]
                let k = ids.len() as f64;
                (0..dims)
                    .map(|dim| {
                        let mut s = 0f64;
                        for &id in ids {
                            s += d[id as usize * dims + dim];
                        }
                        s / k
                    })
                    .collect()
            }
        }
    }
}

impl Database {
    /// Convert a 1-based (negative = from the end) state number to a
    /// 0-based state index, mirroring the upstream query path.
    fn state_index(&self, state: i64) -> Result<usize> {
        let n = self.state_count() as i64;
        let one = if state < 0 { n + state + 1 } else { state };
        if one < 1 || one > n {
            return Err(MiliError::StateOutOfRange(
                usize::try_from(one.max(0)).unwrap_or(0),
                self.state_count(),
            ));
        }
        Ok((one - 1) as usize)
    }

    /// `query("nodpos", "node", labels=…, states=[state])` reduced to
    /// the flat `[label][dim]` buffer for one state, plus the
    /// entity-axis labels in row order and `dims`. `labels = None`
    /// returns every node in node-ordinal order (the row index then
    /// equals the 0-based node id the connectivity columns store).
    /// The buffer keeps its on-disk precision (single-precision plt →
    /// `F32`, double → `F64`) so the centroid arithmetic matches
    /// upstream's numpy dtype exactly.
    fn nodpos_state(
        &self,
        labels: Option<&[i32]>,
        state_idx: usize,
    ) -> Result<(NodposBuf, Vec<i32>, usize)> {
        let deduped: Option<Vec<i32>> = labels.map(|ls| {
            let mut seen: HashSet<i32> = HashSet::new();
            let mut out = Vec::with_capacity(ls.len());
            for &l in ls {
                if seen.insert(l) {
                    out.push(l);
                }
            }
            out
        });
        let args = QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: deduped.as_deref(),
            states: &[state_idx],
            materials: None,
            ips: None,
            subrec: None,
        };
        let (vals, ret_labels) = self.query_with_labels(&args)?;
        let buf = match vals {
            StateValues::F32(d) => NodposBuf::F32(d),
            StateValues::F64(d) => NodposBuf::F64(d),
            _ => {
                return Err(MiliError::MalformedDirectory(
                    "nodpos query returned a non-float buffer",
                ))
            }
        };
        let dims = buf.len() / ret_labels.len().max(1);
        Ok((buf, ret_labels, dims))
    }

    /// Material name(s)/number(s) → the node labels of those materials,
    /// concatenated per material (upstream concatenates
    /// `nodes_of_material` without re-uniquing across materials).
    fn material_node_labels(&self, mesh: MeshId, mats: &[MaterialArg]) -> Result<Vec<i32>> {
        let mut out = Vec::new();
        for m in mats {
            out.extend(self.nodes_of_material(mesh, m)?);
        }
        Ok(out)
    }

    /// `GeometricMeshInfo.compute_centroid`
    /// (`geometric_mesh_info.py:124-154`). `None` mirrors every
    /// upstream `return None` (unknown class / missing label / no
    /// connectivity).
    pub fn gmi_compute_centroid(
        &self,
        mesh: MeshId,
        class: &str,
        label: i32,
        state: i64,
    ) -> Result<Option<Vec<f64>>> {
        let Some(class_labels) = self.labels(mesh, class)? else {
            return Ok(None);
        };
        if !class_labels.contains(&label) {
            return Ok(None);
        }
        if self.superclass_code(mesh, class).is_none() {
            return Ok(None);
        }
        let elem_conns: Vec<i32> = if class == "node" {
            vec![label]
        } else {
            let Some((conn, ncols)) = self.connectivity_ids(mesh, class)? else {
                return Ok(None);
            };
            let code = self
                .superclass_code(mesh, class)
                .ok_or(MiliError::MalformedDirectory("centroid: bad class"))?;
            let beam = code == Superclass::Beam as i32;
            let elem_idx = class_labels
                .iter()
                .position(|&l| l == label)
                .ok_or(MiliError::MalformedDirectory("centroid: label vanished"))?;
            let node_labels = self
                .labels(mesh, "node")?
                .ok_or(MiliError::MalformedDirectory("centroid: no node labels"))?;
            // connectivity_ids row: node id columns then the material
            // column. Drop material (`[:-1]`); for BEAM also drop the
            // 3rd node (`[:-2]`).
            let drop = if beam { 2 } else { 1 };
            let row = &conn[elem_idx * ncols..(elem_idx + 1) * ncols];
            let take = row.len().saturating_sub(drop);
            row[..take]
                .iter()
                .map(|&id| node_labels[id as usize])
                .collect()
        };
        if elem_conns.is_empty() {
            return Ok(None);
        }
        let state_idx = self.state_index(state)?;
        // Upstream `query("nodpos","node",labels=elem_conns)` maps
        // labels to ordinals via `np.isin(labels_of_class, labels)`
        // (`miliinternal.py:1183`): a *membership* test that (a)
        // silently drops requested labels with no subrec coverage and
        // (b) returns the matched rows in ascending node-ordinal order
        // regardless of `elem_conns` order. `np.sum(data[0], axis=0) /
        // float(len(elem_conns))` then sums the matched rows (float32
        // summation is non-associative, so the order is load-bearing)
        // and divides by the full `elem_conns` count (incl. dups).
        let (data, ret, dims) = self.nodpos_state(None, state_idx)?;
        let want: HashSet<i32> = elem_conns.iter().copied().collect();
        let rows: Vec<usize> = (0..ret.len()).filter(|&r| want.contains(&ret[r])).collect();
        let n = elem_conns.len();
        let c: Vec<f64> = match &data {
            NodposBuf::F32(d) => {
                #[allow(clippy::cast_precision_loss)]
                let denom = n as f32;
                (0..dims)
                    .map(|dim| {
                        let mut s = 0f32;
                        for &r in &rows {
                            s += d[r * dims + dim];
                        }
                        f64::from(s / denom)
                    })
                    .collect()
            }
            NodposBuf::F64(d) => {
                #[allow(clippy::cast_precision_loss)]
                let denom = n as f64;
                (0..dims)
                    .map(|dim| {
                        let mut s = 0f64;
                        for &r in &rows {
                            s += d[r * dims + dim];
                        }
                        s / denom
                    })
                    .collect()
            }
        };
        Ok(Some(c))
    }

    /// `GeometricMeshInfo.nearest_node`
    /// (`geometric_mesh_info.py:24-54`). `(label, distance)`;
    /// `(-1, f32::MAX)` when no nodes match (upstream's empty-result
    /// sentinel).
    pub fn gmi_nearest_node(
        &self,
        mesh: MeshId,
        point: &[f64],
        state: i64,
        materials: Option<&[MaterialArg]>,
    ) -> Result<(i32, f64)> {
        let state_idx = self.state_index(state)?;
        // Upstream queries `labels=node_labels` (membership / ascending
        // ordinal, lenient — `miliinternal.py:1183`); equivalently
        // query all nodes then keep the material set in ordinal order.
        let (data, ret, dims) = self.nodpos_state(None, state_idx)?;
        let keep: Option<HashSet<i32>> = match materials {
            Some(mats) => Some(self.material_node_labels(mesh, mats)?.into_iter().collect()),
            None => None,
        };
        let rows: Vec<usize> = (0..ret.len())
            .filter(|&r| match &keep {
                Some(k) => k.contains(&ret[r]),
                None => true,
            })
            .collect();
        if rows.is_empty() {
            return Ok((-1, f64::from(f32::MAX)));
        }
        let mut best = rows[0];
        let mut best_d = f64::INFINITY;
        for &r in &rows {
            let mut s = 0f64;
            for (d, &p) in point.iter().enumerate().take(dims) {
                let diff = data.at_f64(r * dims + d) - p;
                s += diff * diff;
            }
            let dist = s.sqrt();
            if dist < best_d {
                best_d = dist;
                best = r;
            }
        }
        Ok((ret[best], best_d))
    }

    /// `GeometricMeshInfo.nearest_element`
    /// (`geometric_mesh_info.py:56-122`).
    /// `(class_name, label, distance)`; `("", -1, f32::MAX)` when no
    /// class matches the filter.
    pub fn gmi_nearest_element(
        &self,
        mesh: MeshId,
        point: &[f64],
        state: i64,
        materials: Option<&[MaterialArg]>,
        entity_type: Option<&str>,
        superclass: Option<i32>,
    ) -> Result<(String, i32, f64)> {
        let state_idx = self.state_index(state)?;
        // Material names -> numbers (upstream: material_dict.get(str(mat),
        // mat) — names map to their numbers, ints stay as-is).
        let mat_nums: Option<Vec<i32>> = match materials {
            None => None,
            Some(ms) => {
                let map = self.materials()?;
                let mut nums = Vec::new();
                for m in ms {
                    match m {
                        MaterialArg::Num(n) => nums.push(*n),
                        MaterialArg::Name(s) => {
                            if let Some(v) = map.get(s) {
                                nums.extend(v.iter().copied());
                            } else if let Ok(n) = s.parse::<i32>() {
                                nums.push(n);
                            }
                        }
                    }
                }
                Some(nums)
            }
        };
        let (nodpos, _ret, dims) = self.nodpos_state(None, state_idx)?;

        let mut classes: Vec<String> = Vec::new();
        for cn in self.class_names(mesh) {
            if self.connectivity_ids(mesh, &cn)?.is_some() {
                classes.push(cn);
            }
        }
        if let Some(et) = entity_type {
            classes.retain(|c| c == et);
        }
        if let Some(sc) = superclass {
            classes.retain(|c| self.superclass_code(mesh, c) == Some(sc));
        }
        if classes.is_empty() {
            return Ok((String::new(), -1, f64::from(f32::MAX)));
        }

        let big = f64::from(f32::MAX);
        let mut minimums: Vec<(String, usize, f64)> = Vec::new();
        for cn in &classes {
            let Some((conn, ncols)) = self.connectivity_ids(mesh, cn)? else {
                continue;
            };
            let k = ncols - 1; // node id columns (drop material)
            let n_elem = conn.len() / ncols;
            let mut best = 0usize;
            let mut best_d = f64::INFINITY;
            for e in 0..n_elem {
                let row = &conn[e * ncols..(e + 1) * ncols];
                // centroid = sum(nodpos[node_ids], axis=0) / k — in the
                // buffer's numpy dtype, then subtracted from the
                // float64 point (np promotes to float64).
                let centroid = nodpos.centroid(&row[..k], dims);
                let dist = if mat_nums
                    .as_ref()
                    .is_some_and(|mn| !mn.contains(&row[ncols - 1]))
                {
                    big
                } else {
                    let mut s = 0f64;
                    for (&c, &p) in centroid.iter().zip(point.iter()).take(dims) {
                        let diff = c - p;
                        s += diff * diff;
                    }
                    s.sqrt()
                };
                if dist < best_d {
                    best_d = dist;
                    best = e;
                }
            }
            minimums.push((cn.clone(), best, best_d));
        }
        // min by distance; Python `min` keeps the first on ties (class
        // search order).
        let (cls, idx, dist) = minimums
            .into_iter()
            .reduce(|a, b| if b.2 < a.2 { b } else { a })
            .ok_or(MiliError::MalformedDirectory("nearest_element: no class"))?;
        let label = self
            .labels(mesh, &cls)?
            .and_then(|l| l.get(idx).copied())
            .ok_or(MiliError::MalformedDirectory("nearest_element: bad idx"))?;
        Ok((cls, label, dist))
    }

    /// `GeometricMeshInfo.nodes_within_radius`
    /// (`geometric_mesh_info.py:156-190`). Node labels within `radius`
    /// of `center`, in node-ordinal order (or `np.intersect1d` sorted
    /// unique when material-filtered).
    pub fn gmi_nodes_within_radius(
        &self,
        mesh: MeshId,
        center: &[f64],
        radius: f64,
        state: i64,
        materials: Option<&[MaterialArg]>,
    ) -> Result<Vec<i32>> {
        let state_idx = self.state_index(state)?;
        let (data, ret, dims) = self.nodpos_state(None, state_idx)?;
        let mut nodes: Vec<i32> = Vec::new();
        for (r, &lbl) in ret.iter().enumerate() {
            let in_bb = center.iter().take(dims).enumerate().all(|(d, &ctr)| {
                let v = data.at_f64(r * dims + d);
                v >= ctr - radius && v <= ctr + radius
            });
            if !in_bb {
                continue;
            }
            let mut s = 0f64;
            for (d, &ctr) in center.iter().enumerate().take(dims) {
                let diff = data.at_f64(r * dims + d) - ctr;
                s += diff * diff;
            }
            if s.sqrt() <= radius {
                nodes.push(lbl);
            }
        }
        if let Some(mats) = materials {
            // np.intersect1d: sorted unique values common to both.
            let mat_nodes: HashSet<i32> =
                self.material_node_labels(mesh, mats)?.into_iter().collect();
            let here: HashSet<i32> = nodes.iter().copied().collect();
            let mut inter: Vec<i32> = here.intersection(&mat_nodes).copied().collect();
            inter.sort_unstable();
            return Ok(inter);
        }
        Ok(nodes)
    }

    /// `GeometricMeshInfo.elems_of_nodes`
    /// (`geometric_mesh_info.py:192-230`). Element classes (in
    /// connectivity-key order) → sorted-unique element labels touching
    /// any of `node_labels`.
    pub fn gmi_elems_of_nodes(
        &self,
        mesh: MeshId,
        node_labels: &[i32],
        materials: Option<&[MaterialArg]>,
    ) -> Result<Vec<(String, Vec<i32>)>> {
        if node_labels.is_empty() {
            return Ok(vec![]);
        }
        let Some(nodes) = self.labels(mesh, "node")? else {
            return Ok(vec![]);
        };
        let requested: HashSet<i32> = node_labels.iter().copied().collect();
        if !nodes.iter().any(|n| requested.contains(n)) {
            return Ok(vec![]);
        }
        // nlabels = node *ordinals* whose label is requested.
        let nlabels: HashSet<i32> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| requested.contains(n))
            .map(|(i, _)| i as i32)
            .collect();

        let mut out: Vec<(String, Vec<i32>)> = Vec::new();
        for cn in self.elem_conn_classes(mesh)? {
            let Some((conn, ncols)) = self.connectivity_ids(mesh, &cn)? else {
                continue;
            };
            let k = ncols - 1;
            let class_labels = self.labels(mesh, &cn)?.unwrap_or_default();
            let mut rows: Vec<usize> = Vec::new();
            for e in 0..conn.len() / ncols {
                if conn[e * ncols..e * ncols + k]
                    .iter()
                    .any(|&id| nlabels.contains(&id))
                {
                    rows.push(e);
                }
            }
            if rows.is_empty() {
                continue;
            }
            // np.unique(labels[matches]) — sorted unique.
            let mut labs: Vec<i32> = rows
                .iter()
                .filter_map(|&r| class_labels.get(r).copied())
                .collect();
            labs.sort_unstable();
            labs.dedup();
            out.push((cn, labs));
        }

        if let Some(mats) = materials {
            let mut classes_of_mat: Vec<String> = Vec::new();
            for m in mats {
                classes_of_mat.extend(self.material_classes(mesh, m)?);
            }
            out.retain(|(c, _)| classes_of_mat.contains(c));
            for (c, labs) in &mut out {
                let mut of_mat: HashSet<i32> = HashSet::new();
                for m in mats {
                    if let Some(ls) = self.class_labels_of_material(mesh, m, c)? {
                        of_mat.extend(ls);
                    }
                }
                labs.retain(|l| of_mat.contains(l));
            }
        }
        Ok(out)
    }

    /// Element classes with `ELEM_CONNS`, in `connectivity_ids` key
    /// order (`class_names` order, classes that have connectivity).
    fn elem_conn_classes(&self, mesh: MeshId) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for cn in self.class_names(mesh) {
            if self.connectivity_ids(mesh, &cn)?.is_some() {
                out.push(cn);
            }
        }
        Ok(out)
    }

    /// `AdjacencyMapping.mesh_entities_near_coordinate`
    /// (`adjacency.py:73-91`), serial. `elems_of_nodes` of the nodes
    /// in radius, with a trailing `("node", nodes_in_radius)` entry.
    pub fn adj_mesh_entities_near_coordinate(
        &self,
        mesh: MeshId,
        coordinate: &[f64],
        state: i64,
        radius: f64,
        materials: Option<&[MaterialArg]>,
    ) -> Result<ClassLabels> {
        let nodes_in_radius =
            self.gmi_nodes_within_radius(mesh, coordinate, radius, state, materials)?;
        let mut out = self.gmi_elems_of_nodes(mesh, &nodes_in_radius, materials)?;
        out.push(("node".to_owned(), nodes_in_radius));
        Ok(out)
    }

    /// `AdjacencyMapping.mesh_entities_within_radius`
    /// (`adjacency.py:54-71`), serial. `None` = the centroid could not
    /// be computed (upstream raises `ValueError`).
    pub fn adj_mesh_entities_within_radius(
        &self,
        mesh: MeshId,
        class: &str,
        label: i32,
        state: i64,
        radius: f64,
        materials: Option<&[MaterialArg]>,
    ) -> Result<Option<ClassLabels>> {
        let Some(coord) = self.gmi_compute_centroid(mesh, class, label, state)? else {
            return Ok(None);
        };
        Ok(Some(self.adj_mesh_entities_near_coordinate(
            mesh, &coord, state, radius, materials,
        )?))
    }

    /// `AdjacencyMapping.neighbor_elements` (`adjacency.py:157-256`),
    /// serial / `merge_results`.
    pub fn adj_neighbor_elements(
        &self,
        mesh: MeshId,
        class: &str,
        label: i32,
        materials: Option<&[MaterialArg]>,
        neighbor_radius: i64,
    ) -> Result<NeighborElems> {
        let Some(class_labels) = self.labels(mesh, class)? else {
            return Ok(NeighborElems::NoLabels);
        };
        if class_labels.is_empty() {
            return Ok(NeighborElems::NoLabels);
        }
        if !class_labels.contains(&label) {
            return Ok(NeighborElems::LabelMissing);
        }
        let entity_sclass = self.superclass_code(mesh, class);

        let nodes_of = |et: &str, elabels: &[i32]| -> Result<Vec<i32>> {
            match self.nodes_of_elems(mesh, et, elabels)? {
                NodesOfElems::Ok { nodes, .. } => Ok(nodes),
                _ => Ok(vec![]),
            }
        };

        let initial: Vec<i32> = if entity_sclass == Some(Superclass::Node as i32) {
            vec![label]
        } else {
            nodes_of(class, &[label])?
        };

        let mut elements: Vec<(String, Vec<i32>)> = Vec::new();
        let mut processed: HashSet<i32> = HashSet::new();
        let mut to_process: HashSet<i32> = initial.into_iter().collect();
        let mut steps: i64 = 0;
        while !to_process.is_empty() && steps < neighbor_radius {
            for &n in &to_process {
                processed.insert(n);
            }
            let cur: Vec<i32> = to_process.iter().copied().collect();
            let elems = self.gmi_elems_of_nodes(mesh, &cur, None)?;
            dict_merge_concat_unique(&mut elements, &elems);
            to_process.clear();
            steps += 1;
            if steps < neighbor_radius {
                for (ec, el) in &elems {
                    for n in nodes_of(ec, el)? {
                        if !processed.contains(&n) {
                            to_process.insert(n);
                        }
                    }
                }
            }
        }

        if let Some(mats) = materials {
            let mut classes_of_mat: HashSet<String> = HashSet::new();
            for m in mats {
                for c in self.material_classes(mesh, m)? {
                    classes_of_mat.insert(c);
                }
            }
            elements.retain(|(c, _)| classes_of_mat.contains(c));
            for (c, labs) in &mut elements {
                // class_labels_of_material concatenated + np.unique per
                // material (sorted unique).
                let mut of_mat: HashSet<i32> = HashSet::new();
                for m in mats {
                    if let Some(ls) = self.class_labels_of_material(mesh, m, c)? {
                        of_mat.extend(ls);
                    }
                }
                labs.retain(|l| of_mat.contains(l));
            }
        }
        Ok(NeighborElems::Ok(elements))
    }

    /// `AdjacencyMapping.neighbor_nodes` (`adjacency.py:259-313`),
    /// serial. NOTE upstream reassigns `neighbor_nodes` each class
    /// iteration, so only the *last* element class in
    /// `elems_of_nodes`-key order contributes — replicated exactly.
    pub fn adj_neighbor_nodes(&self, mesh: MeshId, class: &str, label: i32) -> Result<Vec<i32>> {
        let entity_sclass = self.superclass_code(mesh, class);
        let nodes_of = |et: &str, elabels: &[i32]| -> Result<(Vec<i32>, usize)> {
            match self.nodes_of_elems(mesh, et, elabels)? {
                NodesOfElems::Ok { nodes, ncols, .. } => Ok((nodes, ncols)),
                _ => Ok((vec![], 0)),
            }
        };

        let node_labels: Vec<i32> = if entity_sclass == Some(Superclass::Node as i32) {
            vec![label]
        } else {
            let (nodes, _) = nodes_of(class, &[label])?;
            if entity_sclass == Some(Superclass::Beam as i32) {
                // ravel then [:-1]
                nodes[..nodes.len().saturating_sub(1)].to_vec()
            } else {
                nodes
            }
        };
        let node_set: HashSet<i32> = node_labels.iter().copied().collect();

        let by_class = self.gmi_elems_of_nodes(mesh, &node_labels, None)?;
        let mut neighbor: Vec<i32> = Vec::new();
        for (cn, class_labels) in &by_class {
            let sc = self
                .superclass_code(mesh, cn)
                .and_then(|c| Superclass::from_code(i64::from(c)))
                .ok_or(MiliError::MalformedDirectory("neighbor_nodes: bad class"))?;
            let Some(nconn) = sc.node_connections() else {
                return Err(MiliError::Unsupported(
                    "neighbor_nodes: superclass has no node-connections map",
                ));
            };
            let (flat, ncols) = nodes_of(cn, class_labels)?;
            if ncols == 0 {
                continue;
            }
            // np.where(np.isin(conn, node_labels)) in C order: iterate
            // rows then columns; gather each match's neighbour columns.
            let mut result: Vec<i32> = Vec::new();
            for e in 0..flat.len() / ncols {
                let row = &flat[e * ncols..(e + 1) * ncols];
                for (col, &v) in row.iter().enumerate() {
                    if node_set.contains(&v) {
                        // Upstream `n_connects[idx2]` IndexErrors when a
                        // matched column exceeds the superclass's
                        // node-connections rows (its own bug for a
                        // BEAM's 3rd/orientation node — `idx2 == 2` vs
                        // the (2,1) M_BEAM map). Mirror that as a typed
                        // error rather than a silent wrong answer.
                        let nc_row = nconn.get(col).ok_or(MiliError::Unsupported(
                            "neighbor_nodes: matched connectivity column \
                             has no node-connections entry",
                        ))?;
                        for &nc in *nc_row {
                            result.push(row[nc]);
                        }
                    }
                }
            }
            neighbor = result;
        }
        // np.setdiff1d(np.unique(neighbor), node_labels): sorted unique
        // minus the searched nodes.
        neighbor.sort_unstable();
        neighbor.dedup();
        neighbor.retain(|n| !node_set.contains(n));
        Ok(neighbor)
    }
}

/// `mili.reductions.dictionary_merge_concat_unique([acc, new])`: for
/// each key concatenate the value arrays then `pd.unique` — which
/// keeps **first-occurrence** order (NOT sorted). New keys are
/// appended in first-seen order.
fn dict_merge_concat_unique(acc: &mut Vec<(String, Vec<i32>)>, new: &[(String, Vec<i32>)]) {
    for (k, v) in new {
        if let Some((_, cur)) = acc.iter_mut().find(|(ek, _)| ek == k) {
            cur.extend(v.iter().copied());
            *cur = unique_first(cur);
        } else {
            acc.push((k.clone(), unique_first(v)));
        }
    }
}

/// `pandas.unique` semantics: unique values in first-occurrence order.
fn unique_first(v: &[i32]) -> Vec<i32> {
    let mut seen: HashSet<i32> = HashSet::new();
    v.iter().copied().filter(|x| seen.insert(*x)).collect()
}
