//! Phase-G primal-only read-surface reshapes.
//!
//! Every method here is a *reshape* of data the core already parses
//! (svar table, srec table, mesh table, params, connectivity) into the
//! shape upstream `_MiliInternal` answers — **no new parity-sensitive
//! math**. Each is bit-exact against the upstream
//! `reference/mili-python/src/mili/miliinternal.py` method it mirrors
//! (cited per-fn); the `mili-py` binding is a thin pass-through that
//! boxes the returned plain data into the upstream-compatible
//! `StateVariable` / `Subrecord` / `MeshObjectClass` dataclasses for
//! `isinstance` / `__eq__` parity.
//!
//! See `planning/mili-py/m4.md` decision 19 (Phase G).

use std::collections::HashMap;

use crate::mesh::Superclass;
use crate::param::{ParamValue, ScalarValue};
use crate::svar::SvarAgg;
use crate::{Database, DirEntryType, MeshId, Result};

/// Mirror of upstream `MeshObjectClass`
/// (`reference/mili-python/src/mili/datatypes.py:457-465`).
#[derive(Debug, Clone)]
pub struct MoClassInfo {
    pub mesh_id: i32,
    pub short_name: String,
    pub long_name: String,
    pub sclass: i32,
    pub elem_qty: i32,
    pub idents_exist: bool,
}

/// Mirror of the `Subrecord` fields the read-path suite compares
/// (`datatypes.py:264-311`, `__eq__`).
#[derive(Debug, Clone)]
pub struct SubrecInfo {
    pub name: String,
    pub class_name: String,
    pub superclass: i32,
    pub organization: i32,
    pub qty_svars: i32,
    pub svar_names: Vec<String>,
    /// Flattened `[start, stop, ...]` ordinal blocks, transformed to
    /// match upstream `Subrecord.ordinal_blocks`
    /// (`afileIO.py:439-459`).
    pub ordinal_blocks: Vec<i64>,
    /// Byte offset of this subrecord within a state's data block,
    /// cumulative over the flattened subrecord list in srec order
    /// (upstream `miliinternal.py:271-272`: `state_byte_offset =
    /// offset; offset += srec.byte_size`).
    pub state_byte_offset: i64,
}

/// Mirror of the `StateVariable` fields the read-path suite compares
/// (`datatypes.py:180-223`, `__eq__`).
#[derive(Debug, Clone)]
pub struct SvarInfo {
    pub name: String,
    pub title: String,
    pub data_type: i32,
    pub agg_type: i32,
    pub list_size: i32,
    pub order: i32,
    pub dims: Vec<i32>,
    pub comp_names: Vec<String>,
    pub containing_svar_names: Vec<String>,
}

/// A single parameter value, in the small set the read-path surface
/// observes (scalars, strings, and 1-D numeric arrays).
#[derive(Debug, Clone)]
pub enum ParamPy {
    Int(i64),
    Float(f64),
    Str(String),
    IntArr(Vec<i64>),
    FloatArr(Vec<f64>),
}

fn agg_code(agg: &SvarAgg) -> i32 {
    match agg {
        SvarAgg::Scalar => 0,
        SvarAgg::Vector { .. } => 1,
        SvarAgg::Array { .. } => 2,
        SvarAgg::VecArray { .. } => 3,
    }
}

fn direct_comps(agg: &SvarAgg) -> &[String] {
    match agg {
        SvarAgg::Vector { comps } | SvarAgg::VecArray { comps, .. } => comps,
        _ => &[],
    }
}

impl Database {
    /// Direct + recursive component names of `name` (the name itself
    /// first), mirroring upstream `StateVariable.recursive_names`
    /// (`datatypes.py:225-228`). Acyclic in every corpus; a visited
    /// guard keeps a malformed cyclic dict from looping.
    fn recursive_svar_names(&self, name: &str, out: &mut Vec<String>, seen: &mut Vec<String>) {
        if seen.iter().any(|s| s == name) {
            return;
        }
        seen.push(name.to_owned());
        out.push(name.to_owned());
        if let Some(sv) = self.svars().get(name) {
            for c in direct_comps(&sv.agg) {
                self.recursive_svar_names(c, out, seen);
            }
        }
    }

    /// For every svar, the ordered-unique class names of the
    /// subrecords it is a member of — directly (its name is in
    /// `Subrecord.svar_names`) or transitively (it is a recursive
    /// component of such a svar). The core analogue of upstream's
    /// `nested_svar.srecs` wiring (`miliinternal.py:240-251`) reduced
    /// to the `srec.class_name` set `classes_of_state_variable` /
    /// `state_variables_of_class` consume.
    fn svar_srec_classes(&self) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for srec in self.srecs().iter() {
            for sub in &srec.subrecords {
                for listed in &sub.svar_names {
                    let mut names = Vec::new();
                    let mut seen = Vec::new();
                    self.recursive_svar_names(listed, &mut names, &mut seen);
                    for n in names {
                        let v = out.entry(n).or_default();
                        if !v.iter().any(|c| c == &sub.mclass) {
                            v.push(sub.mclass.clone());
                        }
                    }
                }
            }
        }
        out
    }

    /// Upstream `containing_svar_names` wiring (`miliinternal.py:146-153`
    /// `addContaining`): for each svar in parse order, every
    /// (recursive) component records that svar as a container, in that
    /// exact append order.
    fn containing_svar_names(&self) -> HashMap<String, Vec<String>> {
        fn add(db: &Database, name: &str, root: &str, out: &mut HashMap<String, Vec<String>>) {
            if let Some(sv) = db.svars().get(name) {
                if matches!(sv.agg, SvarAgg::Vector { .. } | SvarAgg::VecArray { .. }) {
                    for c in direct_comps(&sv.agg).to_vec() {
                        out.entry(c.clone()).or_default().push(root.to_owned());
                        add(db, &c, &c, out);
                    }
                }
            }
        }
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for sv in self.svars().iter() {
            if matches!(sv.agg, SvarAgg::Vector { .. } | SvarAgg::VecArray { .. }) {
                for c in direct_comps(&sv.agg).to_vec() {
                    out.entry(c.clone()).or_default().push(sv.name.clone());
                    add(self, &c, &c, &mut out);
                }
            }
        }
        out
    }

    /// State-record-format quantity. Upstream hardcodes `1` for the
    /// single-fragment reader (`miliinternal.py:236`).
    pub fn srec_fmt_qty(&self) -> i32 {
        1
    }

    /// `superclass_from_class_name` (`miliinternal.py:326-339`):
    /// `None` if the class is unknown (caller maps to
    /// `Superclass.M_INVALID_LABEL`), else the superclass code.
    pub fn superclass_code(&self, mesh: MeshId, class_name: &str) -> Option<i32> {
        self.meshes()
            .mesh(mesh)
            .and_then(|m| m.class(class_name))
            .map(|c| c.superclass as i32)
    }

    /// `mesh_object_classes` (`miliinternal.py:417-423` + the
    /// `elem_qty` / `idents_exist` finalisation at
    /// `miliinternal.py:276-282`). Order = `CLASS_DEF` declaration
    /// order (== `class_names()`).
    pub fn mesh_object_classes(&self, mesh: MeshId) -> Result<Vec<MoClassInfo>> {
        let mut out = Vec::new();
        let Some(m) = self.meshes().mesh(mesh) else {
            return Ok(out);
        };
        for c in m.classes() {
            // Upstream finalisation (`miliinternal.py:276-282`):
            // `idents_exist` ⇔ the class reached finalisation already
            // in `__labels` (a TI label / CLASS_IDENTS source); then
            // `elem_qty = labels.size`, else `elem_qty = conns.rows`.
            let idents_exist = self.idents_exist(mesh, &c.short_name)?;
            let elem_qty = if idents_exist {
                self.labels(mesh, &c.short_name)?.map_or(0, |v| v.len()) as i32
            } else {
                let mut rows = 0usize;
                for &idx in self.meshes().conns_entry_indices(mesh, &c.short_name) {
                    let entry = &self.directory().entries[idx];
                    let conn =
                        crate::mesh::decode_elem_conns(self.a_bytes(), entry, self.header())?;
                    if conn.conn_words > 0 {
                        let words = conn.to_i32_vec()?;
                        rows += words.len() / conn.conn_words;
                    }
                }
                rows as i32
            };
            out.push(MoClassInfo {
                mesh_id: mesh.0,
                short_name: c.short_name.clone(),
                long_name: c.long_name.clone(),
                sclass: c.superclass as i32,
                elem_qty,
                idents_exist,
            });
        }
        Ok(out)
    }

    /// `subrecords` (`miliinternal.py:357-363`) — the parsed subrecord
    /// list, with `superclass` / `ordinal_blocks` derived exactly as
    /// upstream's parser (`afileIO.py:430-462`).
    pub fn subrecords(&self, mesh: MeshId) -> Vec<SubrecInfo> {
        let mut out = Vec::new();
        // Cumulative byte offset within a state's data block, in the
        // flattened srec/subrecord declaration order — the byte-for-byte
        // analogue of upstream's `offset` accumulator
        // (`miliinternal.py:235,271-272`).
        let mut running: i64 = 0;
        for srec in self.srecs().iter() {
            for sub in &srec.subrecords {
                let state_byte_offset = running;
                if let Ok((atoms, widths)) = crate::query::atoms_and_widths(sub, self.svars()) {
                    let size = (sub.object_count() as i64).saturating_mul(
                        crate::srec::derive_lumps(&atoms, &widths).bytes_per_object() as i64,
                    );
                    running = running.saturating_add(size);
                }
                let sclass = self
                    .meshes()
                    .mesh(mesh)
                    .and_then(|m| m.class(&sub.mclass))
                    .map_or(-1, |c| c.superclass as i32);
                let ordinal_blocks = if sclass == Superclass::Mesh as i32 {
                    vec![0i64, 1]
                } else {
                    let mut v = Vec::with_capacity(sub.id_blocks.len() * 2);
                    for &(s, e) in &sub.id_blocks {
                        // upstream: stop += 1 (odd idx) then -1 all →
                        // (start-1, stop). (afileIO.py:443-445)
                        v.push(i64::from(s) - 1);
                        v.push(i64::from(e));
                    }
                    // upstream sorts blocks by start if not already
                    // monotonically increasing (afileIO.py:451-459).
                    let monotone = v.windows(2).all(|w| w[1] >= w[0]);
                    if !monotone {
                        let mut pairs: Vec<(i64, i64)> =
                            v.chunks_exact(2).map(|c| (c[0], c[1])).collect();
                        pairs.sort_by_key(|p| p.0);
                        v = pairs.into_iter().flat_map(|(a, b)| [a, b]).collect();
                    }
                    v
                };
                out.push(SubrecInfo {
                    name: sub.name.clone(),
                    class_name: sub.mclass.clone(),
                    superclass: sclass,
                    organization: sub.organization as i32,
                    qty_svars: sub.svar_names.len() as i32,
                    svar_names: sub.svar_names.clone(),
                    ordinal_blocks,
                    state_byte_offset,
                });
            }
        }
        out
    }

    /// `state_variables` (`miliinternal.py:497-503`) — every svar in
    /// parse order, with the `__eq__`-relevant fields and the wired
    /// `containing_svar_names`.
    pub fn state_variables(&self) -> Vec<SvarInfo> {
        let containing = self.containing_svar_names();
        let mut out = Vec::new();
        for sv in self.svars().iter() {
            let (dims, comp_names): (Vec<i32>, Vec<String>) = match &sv.agg {
                SvarAgg::Scalar => (vec![], vec![]),
                SvarAgg::Vector { comps } => (vec![], comps.clone()),
                SvarAgg::Array { dims } => (dims.clone(), vec![]),
                SvarAgg::VecArray { dims, comps } => (dims.clone(), comps.clone()),
            };
            out.push(SvarInfo {
                name: sv.name.clone(),
                title: sv.title.clone(),
                data_type: sv.type_code,
                agg_type: agg_code(&sv.agg),
                list_size: comp_names.len() as i32,
                order: dims.len() as i32,
                dims,
                comp_names,
                containing_svar_names: containing.get(&sv.name).cloned().unwrap_or_default(),
            });
        }
        out
    }

    /// `queriable_svars(vector_only, show_ips)`
    /// (`miliinternal.py:505-529`) — pure reshape of the svar table.
    pub fn queriable_svars(&self, vector_only: bool, show_ips: bool) -> Vec<String> {
        let mut q = Vec::new();
        for sv in self.svars().iter() {
            match &sv.agg {
                SvarAgg::Vector { comps } => {
                    q.push(sv.name.clone());
                    for c in comps {
                        q.push(format!("{}[{}]", sv.name, c));
                    }
                }
                SvarAgg::VecArray { dims, comps } => {
                    for c in comps {
                        let ip = if show_ips {
                            format!("[0-{}]", dims[0])
                        } else {
                            String::new()
                        };
                        q.push(format!("{}{ip}[{c}]", sv.name));
                    }
                }
                _ => {
                    if !vector_only {
                        q.push(sv.name.clone());
                    }
                }
            }
        }
        q
    }

    /// `classes_of_state_variable` (`miliinternal.py:717-733`).
    /// `None` when the svar is unknown (caller emits the upstream
    /// error + empty list).
    pub fn classes_of_state_variable(&self, svar: &str) -> Option<Vec<String>> {
        self.svars().get(svar)?;
        Some(
            self.svar_srec_classes()
                .get(svar)
                .cloned()
                .unwrap_or_default(),
        )
    }

    /// `state_variables_of_class` (`miliinternal.py:735-755`).
    /// `None` when the class is unknown.
    pub fn state_variables_of_class(&self, mesh: MeshId, class_name: &str) -> Option<Vec<String>> {
        self.meshes().mesh(mesh)?.class(class_name)?;
        let by_svar = self.svar_srec_classes();
        let mut out = Vec::new();
        for sv in self.svars().iter() {
            if by_svar
                .get(&sv.name)
                .is_some_and(|cs| cs.iter().any(|c| c == class_name))
                && !out.iter().any(|n| n == &sv.name)
            {
                out.push(sv.name.clone());
            }
        }
        Some(out)
    }

    /// `containing_state_variables_of_class` (`miliinternal.py:769-794`).
    /// `None` when the svar or class is unknown.
    pub fn containing_state_variables_of_class(
        &self,
        mesh: MeshId,
        svar: &str,
        class_name: &str,
    ) -> Option<Vec<String>> {
        self.svars().get(svar)?;
        self.meshes().mesh(mesh)?.class(class_name)?;
        let containing = self.containing_svar_names();
        let by_svar = self.svar_srec_classes();
        let potential = containing.get(svar).cloned().unwrap_or_default();
        Some(
            potential
                .into_iter()
                .filter(|c| {
                    by_svar
                        .get(c)
                        .is_some_and(|cs| cs.iter().any(|x| x == class_name))
                })
                .collect(),
        )
    }

    /// `components_of_vector_svar` (`miliinternal.py:796-813`).
    /// `Ok(None)` = unknown svar; `Err(())` = svar is not a vector
    /// (the two distinct upstream error paths the caller renders).
    #[allow(clippy::result_unit_err)]
    pub fn components_of_vector_svar(&self, svar: &str) -> std::result::Result<Vec<String>, bool> {
        match self.svars().get(svar) {
            None => Err(false), // does not exist
            Some(sv) => match &sv.agg {
                SvarAgg::Vector { comps } | SvarAgg::VecArray { comps, .. } => Ok(comps.clone()),
                _ => Err(true), // exists but not a vector
            },
        }
    }

    /// `state_variable_titles` (`miliinternal.py:757-767`). The derived
    /// title merge is Phase H; the primal half is `{name: title}` in
    /// parse order.
    pub fn state_variable_titles(&self) -> Vec<(String, String)> {
        self.svars()
            .iter()
            .map(|s| (s.name.clone(), s.title.clone()))
            .collect()
    }

    /// `int_points_of_state_variable` (`miliinternal.py:429-453`).
    /// `None` = unknown svar or class; otherwise the IP labels (the
    /// element-set payload minus its trailing count) for the parent
    /// element-set whose class set contains `class_name`.
    pub fn int_points_of_state_variable(
        &self,
        mesh: MeshId,
        svar_name: &str,
        class_name: &str,
    ) -> Option<Vec<i32>> {
        self.svars().get(svar_name)?;
        self.meshes().mesh(mesh)?.class(class_name)?;
        let ip = self.build_int_points();
        let by_svar = self.svar_srec_classes();
        let mut out: Vec<i32> = Vec::new();
        for parent in ip.parents_of(svar_name) {
            let in_class = by_svar
                .get(&parent.es_svar)
                .is_some_and(|cs| cs.iter().any(|c| c == class_name));
            if in_class {
                out = parent
                    .payload
                    .split_last()
                    .map(|(_, head)| head.to_vec())
                    .unwrap_or_default();
            }
        }
        Some(out)
    }

    /// Per-class connectivity columns `(material, part)` per element,
    /// concatenated across the class's `ELEM_CONNS` entries in
    /// directory order (the raw rows upstream's `__elems_of_mat` /
    /// `__elems_of_part` are derived from, `miliinternal.py:213-219`).
    fn conn_mat_part(&self, mesh: MeshId, class_name: &str) -> Result<Vec<(i32, i32)>> {
        let mut out = Vec::new();
        for &idx in self.meshes().conns_entry_indices(mesh, class_name) {
            let entry = &self.directory().entries[idx];
            let conn = crate::mesh::decode_elem_conns(self.a_bytes(), entry, self.header())?;
            let words = conn.conn_words;
            if words < 2 {
                continue;
            }
            let raw = conn.to_i32_vec()?;
            for row in raw.chunks_exact(words) {
                out.push((row[words - 2], row[words - 1]));
            }
        }
        Ok(out)
    }

    /// `materials_of_class_name` (`miliinternal.py:836-855`) — the
    /// per-element material number (`-1` where unknown, matching the
    /// upstream `np.zeros … [:] = -1` seed).
    pub fn materials_of_class_name(
        &self,
        mesh: MeshId,
        class_name: &str,
    ) -> Result<Option<Vec<i32>>> {
        if self
            .meshes()
            .mesh(mesh)
            .and_then(|m| m.class(class_name))
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(self.per_elem_column(mesh, class_name, true)?))
    }

    /// `parts_of_class_name` (`miliinternal.py:815-834`).
    pub fn parts_of_class_name(&self, mesh: MeshId, class_name: &str) -> Result<Option<Vec<i32>>> {
        if self
            .meshes()
            .mesh(mesh)
            .and_then(|m| m.class(class_name))
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(self.per_elem_column(mesh, class_name, false)?))
    }

    /// Per-element material (`material=true`) or part column, sized to
    /// the class's *label* count and seeded with `-1`, then filled from
    /// connectivity — exactly upstream's `np.zeros(labels.shape);
    /// [:] = -1; elem_x[idxs] = x` (`miliinternal.py:824-855`). Classes
    /// without connectivity (glob / mat / node) stay all `-1`.
    fn per_elem_column(&self, mesh: MeshId, class_name: &str, material: bool) -> Result<Vec<i32>> {
        let n = self.labels(mesh, class_name)?.map_or(0, |v| v.len());
        let mut out = vec![-1i32; n];
        for (i, (m, p)) in self
            .conn_mat_part(mesh, class_name)?
            .into_iter()
            .enumerate()
        {
            if i < out.len() {
                out[i] = if material { m } else { p };
            }
        }
        Ok(out)
    }

    /// Material number → ordered class names that own elements of it,
    /// keyed in element-connectivity discovery order. The core
    /// analogue of upstream's `__elems_of_mat` keyset
    /// (`miliinternal.py:213-217`); drives `material_classes` /
    /// `all_labels_of_material`.
    fn elems_of_mat_classes(&self, mesh: MeshId) -> Result<Vec<(i32, Vec<String>)>> {
        let mut order: Vec<i32> = Vec::new();
        let mut map: HashMap<i32, Vec<String>> = HashMap::new();
        let Some(m) = self.meshes().mesh(mesh) else {
            return Ok(vec![]);
        };
        for c in m.classes() {
            let cmp = self.conn_mat_part(mesh, &c.short_name)?;
            let mut mats: Vec<i32> = cmp.iter().map(|&(mat, _)| mat).collect();
            mats.sort_unstable();
            mats.dedup();
            for mat in mats {
                let e = map.entry(mat).or_insert_with(|| {
                    order.push(mat);
                    Vec::new()
                });
                if !e.iter().any(|x| x == &c.short_name) {
                    e.push(c.short_name.clone());
                }
            }
        }
        Ok(order
            .into_iter()
            .map(|mat| (mat, map.remove(&mat).unwrap_or_default()))
            .collect())
    }

    /// Resolve a material name / number / digit-string to the material
    /// numbers it covers (upstream `material_classes` /
    /// `all_labels_of_material` head, `miliinternal.py:692-715`).
    /// `None` = the material does not exist.
    fn resolve_material_nums(&self, mat: &MaterialArg) -> Result<Option<Vec<i32>>> {
        let mats = self.materials()?;
        match mat {
            MaterialArg::Name(s) => {
                if let Some(nums) = mats.get(s) {
                    return Ok(Some(nums.clone()));
                }
                if let Ok(n) = s.parse::<i32>() {
                    return Ok(Some(vec![n]));
                }
                Ok(None)
            }
            MaterialArg::Num(n) => Ok(Some(vec![*n])),
        }
    }

    /// `material_classes` (`miliinternal.py:692-715`). `None` only on
    /// an invalid material *type* (the binding pre-validates); an
    /// unknown-but-typed material yields `Some(vec![])`.
    pub fn material_classes(&self, mesh: MeshId, mat: &MaterialArg) -> Result<Vec<String>> {
        let Some(nums) = self.resolve_material_nums(mat)? else {
            return Ok(vec![]);
        };
        let by_mat = self.elems_of_mat_classes(mesh)?;
        let mut out = Vec::new();
        for n in nums {
            if let Some((_, classes)) = by_mat.iter().find(|(m, _)| *m == n) {
                out.extend(classes.iter().cloned());
            }
        }
        Ok(out)
    }

    /// `class_labels_of_material` (`miliinternal.py:857-893`). `None` =
    /// the class does not exist (upstream returns an empty array +
    /// error); `Some` is the labels of that class with the material.
    pub fn class_labels_of_material(
        &self,
        mesh: MeshId,
        mat: &MaterialArg,
        class_name: &str,
    ) -> Result<Option<Vec<i32>>> {
        let Some(labels) = self.labels(mesh, class_name)? else {
            return Ok(None);
        };
        let nums = self.resolve_material_nums(mat)?.unwrap_or_default();
        let cmp = self.conn_mat_part(mesh, class_name)?;
        let mut out = Vec::new();
        for n in &nums {
            for (i, &(m, _)) in cmp.iter().enumerate() {
                if m == *n {
                    if let Some(&lbl) = labels.get(i) {
                        out.push(lbl);
                    }
                }
            }
        }
        Ok(Some(out))
    }

    /// `all_labels_of_material` (`miliinternal.py:895-918`): every
    /// class with the material → that class's matching labels.
    pub fn all_labels_of_material(
        &self,
        mesh: MeshId,
        mat: &MaterialArg,
    ) -> Result<Vec<(String, Vec<i32>)>> {
        let Some(nums) = self.resolve_material_nums(mat)? else {
            return Ok(vec![]);
        };
        let by_mat = self.elems_of_mat_classes(mesh)?;
        let mut out = Vec::new();
        for n in nums {
            if let Some((_, classes)) = by_mat.iter().find(|(m, _)| *m == n) {
                for cls in classes {
                    let lbls = self
                        .class_labels_of_material(mesh, &MaterialArg::Num(n), cls)?
                        .unwrap_or_default();
                    out.push((cls.clone(), lbls));
                }
            }
        }
        Ok(out)
    }

    /// Upstream `__params` dict reconstruction
    /// (`miliinternal.py:107-124`): the selected `TI_PARAM` subset
    /// (`MAT_NAME_<n>`, `SetRGB*`, `particles_on` / `code_name` /
    /// `nproc`) then *every* `MILI_PARAM` then *every*
    /// `APPLICATION_PARAM`, in directory order, last write winning the
    /// value while keeping first-seen key position (CPython dict
    /// semantics).
    fn params_dict(&self) -> Result<Vec<(String, ParamPy)>> {
        let mut order: Vec<String> = Vec::new();
        let mut map: HashMap<String, ParamPy> = HashMap::new();
        let mut put = |k: String, v: ParamPy, order: &mut Vec<String>| {
            if !map.contains_key(&k) {
                order.push(k.clone());
            }
            map.insert(k, v);
        };
        let header = self.header();
        for entry in &self.directory().entries {
            if entry.name_count == 0 {
                continue;
            }
            let name = self
                .directory()
                .names
                .get(entry.name_start as usize)
                .to_owned();
            match entry.entry_type {
                DirEntryType::TiParam => {
                    if let Some(rest) = name.strip_prefix("MAT_NAME_") {
                        if rest.parse::<i32>().is_ok() {
                            if let ParamValue::String(s) =
                                ParamValue::decode(self.a_bytes(), entry, header)?
                            {
                                put(name.clone(), ParamPy::Str(s.to_owned()), &mut order);
                            }
                        }
                    } else if name.starts_with("SetRGB")
                        || name == "particles_on"
                        || name == "code_name"
                        || name == "nproc"
                    {
                        if let Some(v) = decode_param_py(self.a_bytes(), entry, header)? {
                            put(name.clone(), v, &mut order);
                        }
                    }
                }
                DirEntryType::MiliParam | DirEntryType::ApplicationParam => {
                    if let Some(v) = decode_param_py(self.a_bytes(), entry, header)? {
                        put(name.clone(), v, &mut order);
                    }
                }
                _ => {}
            }
        }
        Ok(order
            .into_iter()
            .map(|k| {
                let v = map.remove(&k).expect("key present");
                (k, v)
            })
            .collect())
    }

    /// `parameters` (`miliinternal.py:365-371`).
    pub fn parameters(&self) -> Result<Vec<(String, ParamPy)>> {
        self.params_dict()
    }

    /// `parameter(name)` (`miliinternal.py:373-383`) — `None` lets the
    /// binding apply the caller's default.
    pub fn parameter(&self, name: &str) -> Result<Option<ParamPy>> {
        Ok(self
            .params_dict()?
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v))
    }

    /// `metadata` (`miliinternal.py:126-135`) — the seven-field
    /// dictionary, each field defaulting exactly as upstream when its
    /// backing param is absent.
    pub fn metadata(&self) -> Result<Metadata> {
        let s = |n: &str| -> Result<Option<String>> {
            Ok(match self.parameter(n)? {
                Some(ParamPy::Str(v)) => Some(v),
                _ => None,
            })
        };
        let nprocs = match self.parameter("nproc")? {
            Some(ParamPy::Int(n)) => n as i32,
            _ => 1,
        };
        Ok(Metadata {
            code_name: s("code_name")?.unwrap_or_default(),
            username: s("username")?.unwrap_or_default(),
            job_id: s("job_id")?.unwrap_or_default(),
            nprocs,
            date: s("date")?.unwrap_or_default(),
            host_name: s("host name")?.unwrap_or_default(),
            library_version: s("lib version")?.unwrap_or_default(),
        })
    }

    // ---- derived-variable enumeration ----
    //
    // Mirrors `mili.derived.DerivedExpressions`
    // (`reference/mili-python/src/mili/derived.py`) and the three
    // `_MiliInternal` pass-throughs (`miliinternal.py:531-565`). This
    // is the *enumeration* surface only — which derived results exist
    // and on which classes — built by composing the already
    // parity-gated primal reshapes (`queriable_svars`,
    // `classes_of_state_variable`, `mesh_object_classes`,
    // `class_names`) with the static spec registry below. No derived
    // *math* is ported here (that stays the `derived.rs` `*_spec`
    // compute path); no new file parsing — a reshape, not a re-port.

    /// `_MiliInternal.supported_derived_variables`
    /// (`miliinternal.py:531` → `derived.py:603`
    /// `list(self.__derived_expressions.keys())`). The static set of
    /// derived names the library can compute *if* the required primals
    /// exist — registry insertion order.
    #[must_use]
    pub fn supported_derived_variables(&self) -> Vec<String> {
        DERIVED_REGISTRY
            .iter()
            .map(|e| e.name.to_string())
            .collect()
    }

    /// `DerivedExpressions.__variable_exists_for_class`
    /// (`derived.py:615-621`): `class_name in
    /// classes_of_state_variable(variable)` OR (KeyError-guarded)
    /// `class_name in classes_of_derived_variable(variable)`. A primal
    /// req is never itself a registry name in the current spec, so the
    /// derived branch is a terminating `Err`-as-`false` — no recursion.
    fn derived_var_exists_for_class(&self, mesh: MeshId, variable: &str, class_name: &str) -> bool {
        let primal_exists = self
            .classes_of_state_variable(variable)
            .is_some_and(|cs| cs.iter().any(|c| c == class_name));
        let derived_exists = self
            .classes_of_derived_variable(mesh, variable)
            .is_ok_and(|cs| cs.iter().any(|c| c == class_name));
        primal_exists || derived_exists
    }

    /// `_MiliInternal.derived_variables_of_class`
    /// (`miliinternal.py:539` → `derived.py:624-657`). Empty when the
    /// class is unknown (upstream returns `[]`). Registry order.
    #[must_use]
    pub fn derived_variables_of_class(&self, mesh: MeshId, class_name: &str) -> Vec<String> {
        let mut out = Vec::new();
        if !self.class_names(mesh).iter().any(|c| c == class_name) {
            return out;
        }
        let Ok(mocs) = self.mesh_object_classes(mesh) else {
            return out;
        };
        let Some(cdef) = mocs.iter().find(|c| c.short_name == class_name) else {
            return out;
        };
        let sclass = cdef.sclass;
        let queriable = self.queriable_svars(false, false);
        let in_q = |n: &str| {
            queriable.iter().any(|s| s == n) || DERIVED_REGISTRY.iter().any(|e| e.name == n)
        };
        for spec in DERIVED_REGISTRY {
            if !spec.only_sclasses.is_empty() && !spec.only_sclasses.contains(&sclass) {
                continue;
            }
            // `primals_found` only grows when *both* the
            // queriable/registry gate and the class-existence gate
            // pass, so a full count == `primals.len()` is upstream's
            // `len(primals_found) == len(spec['primals']) and all(...)`.
            let count = |reqs: &[&str]| -> usize {
                reqs.iter()
                    .zip(spec.primals_class.iter())
                    .filter(|(rp, rpc)| {
                        in_q(rp)
                            && self.derived_var_exists_for_class(
                                mesh,
                                rp,
                                rpc.unwrap_or(class_name),
                            )
                    })
                    .count()
            };
            if count(spec.primals) == spec.primals.len() {
                out.push(spec.name.to_string());
            } else if !spec.alternate_primals.is_empty()
                && count(spec.alternate_primals) == spec.primals.len()
            {
                // Upstream gates the alt branch on `len(spec['primals'])`
                // (not `alternate_primals`); equal-length in the spec.
                out.push(spec.name.to_string());
            }
        }
        out
    }

    /// `_MiliInternal.classes_of_derived_variable`
    /// (`miliinternal.py:553` → `derived.py:660-690`). `Err` when the
    /// derived name is unknown (upstream raises `KeyError`). Class
    /// order = `mesh_object_classes`.
    pub fn classes_of_derived_variable(&self, mesh: MeshId, var_name: &str) -> Result<Vec<String>> {
        let Some(spec) = DERIVED_REGISTRY.iter().find(|e| e.name == var_name) else {
            return Err(crate::MiliError::UnknownSvar(var_name.to_string()));
        };
        let mocs = self.mesh_object_classes(mesh)?;
        let all_same_class = spec.primals_class.iter().all(Option::is_none);
        let mut out = Vec::new();
        for c in &mocs {
            if !spec.only_sclasses.is_empty() && !spec.only_sclasses.contains(&c.sclass) {
                continue;
            }
            let ok = if all_same_class {
                // CASE 1 (`derived.py:677-683`): every primal on the
                // result's own class.
                spec.primals
                    .iter()
                    .all(|p| self.derived_var_exists_for_class(mesh, p, &c.short_name))
            } else {
                // CASE 2 (`derived.py:685-690`): each primal on its
                // declared class (never `None` in a CASE-2 spec).
                spec.primals
                    .iter()
                    .zip(spec.primals_class.iter())
                    .all(|(p, pc)| {
                        self.derived_var_exists_for_class(
                            mesh,
                            p,
                            pc.unwrap_or(c.short_name.as_str()),
                        )
                    })
            };
            if ok {
                out.push(c.short_name.clone());
            }
        }
        Ok(out)
    }
}

/// One `DerivedExpressions.__derived_expressions` entry, reduced to the
/// fields the *enumeration* accessors read (`derived.py:25-34`
/// `DerivedSpec`): the compute fn / `supports_batching` are the
/// compute path's concern (`derived.rs`), not enumeration's.
struct DerivedRegEntry {
    name: &'static str,
    /// `spec['primals']` — required primal svar names.
    primals: &'static [&'static str],
    /// `spec['alternate_primals']` (empty = key absent).
    alternate_primals: &'static [&'static str],
    /// `spec['primals_class']`, `None` = "same class as the result"
    /// (upstream's `None`); length always == `primals`.
    primals_class: &'static [Option<&'static str>],
    /// `spec['only_sclasses']` superclass codes (empty = key absent).
    only_sclasses: &'static [i32],
}

// Shared column shapes (upstream repeats these verbatim per entry).
const PC1: &[Option<&str>] = &[None];
const PC2: &[Option<&str>] = &[None, None];
const PC3: &[Option<&str>] = &[None, None, None];
const PC6: &[Option<&str>] = &[None, None, None, None, None, None];
const NODE1: &[Option<&str>] = &[Some("node")];
const NODE3: &[Option<&str>] = &[Some("node"), Some("node"), Some("node")];
const STRAIN6: &[&str] = &["ex", "ey", "ez", "exy", "eyz", "ezx"];
const STRESS6: &[&str] = &["sx", "sy", "sz", "sxy", "syz", "szx"];
const NO_ALT: &[&str] = &[];
const NO_SC: &[i32] = &[];
const SC_HEX_TET: &[i32] = &[Superclass::Hex as i32, Superclass::Tet as i32];
const SC_QUAD: &[i32] = &[Superclass::Quad as i32];
const SC_HEX: &[i32] = &[Superclass::Hex as i32];

/// Faithful transcription of `derived.py`'s `__derived_expressions`
/// dict (insertion order; `derived.py:56-600`). Enum `.value` strings
/// extracted from the installed `mili.mdg_defines` oracle. The
/// corpus-wide `parity_derived_enum.rs` gate validates this table
/// transitively against `_MiliInternal`.
#[rustfmt::skip]
static DERIVED_REGISTRY: &[DerivedRegEntry] = &[
    DerivedRegEntry { name: "disp_x", primals: &["ux"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "disp_y", primals: &["uy"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "disp_z", primals: &["uz"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "disp_mag", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: PC3, only_sclasses: NO_SC },
    DerivedRegEntry { name: "disp_rad_mag_xy", primals: &["ux", "uy"], alternate_primals: NO_ALT, primals_class: PC2, only_sclasses: NO_SC },
    DerivedRegEntry { name: "vel_x", primals: &["ux"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "vel_y", primals: &["uy"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "vel_z", primals: &["uz"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "acc_x", primals: &["ux"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "acc_y", primals: &["uy"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "acc_z", primals: &["uz"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "vol_strain", primals: &["ex", "ey", "ez"], alternate_primals: NO_ALT, primals_class: PC3, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_strain1", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_strain2", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_strain3", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_strain1", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_strain2", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_strain3", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_strain1_alt", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_strain2_alt", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_strain3_alt", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_strain1_alt", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_strain2_alt", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_strain3_alt", primals: STRAIN6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_stress1", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_stress2", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_stress3", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "eff_stress", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "pressure", primals: &["sx", "sy", "sz"], alternate_primals: NO_ALT, primals_class: PC3, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_stress1", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_stress2", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "prin_dev_stress3", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "max_shear_stress", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "triaxiality", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "norm_press", primals: STRESS6, alternate_primals: NO_ALT, primals_class: PC6, only_sclasses: NO_SC },
    DerivedRegEntry { name: "eps_rate", primals: &["eps"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "nodtangmag", primals: &["nodtang_x", "nodtang_y", "nodtang_z"], alternate_primals: NO_ALT, primals_class: PC3, only_sclasses: NO_SC },
    DerivedRegEntry { name: "mat_cog_disp_x", primals: &["matcgx"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "mat_cog_disp_y", primals: &["matcgy"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "mat_cog_disp_z", primals: &["matcgz"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "element_volume", primals: &["nodpos"], alternate_primals: NO_ALT, primals_class: NODE1, only_sclasses: SC_HEX_TET },
    DerivedRegEntry { name: "area", primals: &["nodpos"], alternate_primals: NO_ALT, primals_class: NODE1, only_sclasses: SC_QUAD },
    DerivedRegEntry { name: "centroid", primals: &["nodpos"], alternate_primals: NO_ALT, primals_class: NODE1, only_sclasses: NO_SC },
    DerivedRegEntry { name: "surfstrainx", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: NODE3, only_sclasses: SC_HEX },
    DerivedRegEntry { name: "surfstrainy", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: NODE3, only_sclasses: SC_HEX },
    DerivedRegEntry { name: "surfstrainz", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: NODE3, only_sclasses: SC_HEX },
    DerivedRegEntry { name: "surfstrainxy", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: NODE3, only_sclasses: SC_HEX },
    DerivedRegEntry { name: "surfstrainyz", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: NODE3, only_sclasses: SC_HEX },
    DerivedRegEntry { name: "surfstrainzx", primals: &["ux", "uy", "uz"], alternate_primals: NO_ALT, primals_class: NODE3, only_sclasses: SC_HEX },
    DerivedRegEntry { name: "relative_volume", primals: &["nodpos"], alternate_primals: NO_ALT, primals_class: NODE1, only_sclasses: SC_HEX_TET },
    DerivedRegEntry { name: "normal_force", primals: &["sn"], alternate_primals: &["nodpres"], primals_class: PC1, only_sclasses: SC_QUAD },
    DerivedRegEntry { name: "force_x", primals: &["s1"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: SC_QUAD },
    DerivedRegEntry { name: "force_y", primals: &["s2"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: SC_QUAD },
    DerivedRegEntry { name: "force_z", primals: &["s3"], alternate_primals: NO_ALT, primals_class: PC1, only_sclasses: SC_QUAD },
    DerivedRegEntry { name: "shear_magnitude", primals: &["qxx", "qyy"], alternate_primals: NO_ALT, primals_class: PC2, only_sclasses: NO_SC },
];

/// A `material=` argument: a name / digit-string, or an integer.
#[derive(Debug, Clone)]
pub enum MaterialArg {
    Name(String),
    Num(i32),
}

/// Mirror of upstream `Metadata` (`datatypes.py:96-104`).
#[derive(Debug, Clone)]
pub struct Metadata {
    pub code_name: String,
    pub username: String,
    pub job_id: String,
    pub nprocs: i32,
    pub date: String,
    pub host_name: String,
    pub library_version: String,
}

fn decode_param_py(
    bytes: &[u8],
    entry: &crate::DirEntry,
    header: crate::Header,
) -> Result<Option<ParamPy>> {
    Ok(Some(match ParamValue::decode(bytes, entry, header)? {
        ParamValue::Scalar(ScalarValue::I32(n)) => ParamPy::Int(i64::from(n)),
        ParamValue::Scalar(ScalarValue::I64(n)) => ParamPy::Int(n),
        ParamValue::Scalar(ScalarValue::F32(n)) => ParamPy::Float(f64::from(n)),
        ParamValue::Scalar(ScalarValue::F64(n)) => ParamPy::Float(n),
        ParamValue::String(s) => ParamPy::Str(s.to_owned()),
        ParamValue::Array(arr) => {
            use crate::param::DataType;
            match arr.data_type {
                DataType::Int | DataType::Int4 | DataType::Int8 => {
                    let mut v: Vec<i64> = Vec::new();
                    let bswap = !header.is_native_endian();
                    if matches!(arr.data_type, DataType::Int8) {
                        crate::endian::for_each_swap::<i64, _>(arr.data, bswap, |x| v.push(x));
                    } else {
                        crate::endian::for_each_swap::<i32, _>(arr.data, bswap, |x| {
                            v.push(i64::from(x));
                        });
                    }
                    ParamPy::IntArr(v)
                }
                _ => {
                    let mut v: Vec<f64> = Vec::new();
                    let bswap = !header.is_native_endian();
                    if matches!(arr.data_type, DataType::Float8) {
                        crate::endian::for_each_swap::<f64, _>(arr.data, bswap, |x| v.push(x));
                    } else {
                        crate::endian::for_each_swap::<f32, _>(arr.data, bswap, |x| {
                            v.push(f64::from(x));
                        });
                    }
                    ParamPy::FloatArr(v)
                }
            }
        }
    }))
}
