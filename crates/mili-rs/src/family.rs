//! `Database` — the opened-mili-family handle.
//!
//! `Database::open(path)` is the user's entry point. Given a path to a
//! family's `.A` file (e.g. `data/serial/basic1/basic1.pltA`) it:
//!
//! 1. Memory-maps the `.A` file so byte ranges from directory entries
//!    remain valid for the database's lifetime.
//! 2. Parses the [`Header`] and [`Directory`].
//! 3. Builds the [`ParamTable`] index over the directory's
//!    `MILI_PARAM` / `APPLICATION_PARAM` / `TI_PARAM` entries.
//! 4. Loads the state map — inline from the `.A` trailer or from the
//!    sibling `<root>T` tfile, whichever the layout dictates
//!    (`StateMapSource::pick`).
//!
//! Lazy state-file mapping is not yet implemented; that lands when
//! the read path (`query.rs`) needs it. The family root is stashed so
//! state files (`<root>00`, `<root>01`, …) can be resolved by index.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use rayon::prelude::*;

use crate::directory::{DirEntry, DirEntryType, Directory};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};
use crate::mesh::{self, Connectivity, MaterialId, MeshId, MeshTable, Nodes};
use crate::param::{DataType, ParamTable, ParamValue, ScalarValue};
use crate::query::{plan_state_svar_ip, Filter, IntPoints, QueryResult, ReadPlan, StateValues};
use crate::srec::SrecTable;
use crate::state::{self, StateMapSource, StateMeta};
use crate::svar::{NumType, SvarAgg, SvarTable};

/// An opened mili family — the read-side handle through which all
/// parsed metadata and state byte ranges are reachable.
pub struct Database {
    a_path: PathBuf,
    a_mmap: Mmap,
    header: Header,
    directory: Directory,
    params: ParamTable,
    meshes: MeshTable,
    svars: SvarTable,
    srecs: SrecTable,
    states: Vec<StateMeta>,
    state_mmaps: Mutex<HashMap<i32, Arc<Mmap>>>,
}

impl Database {
    /// Open a mili family given the path to its `.A` file.
    ///
    /// Sibling state files (`<root>00`, `<root>01`, …) are not mapped
    /// at open time. The tfile (`<root>T`) is read once if the
    /// directory indicates the state map lives there; otherwise it is
    /// not opened.
    pub fn open(a_path: impl AsRef<Path>) -> Result<Self> {
        let a_path = a_path.as_ref().to_path_buf();
        let a_mmap = mmap_read_only(&a_path)?;

        let header = Header::parse(&a_mmap)?;
        let directory = Directory::parse(&a_mmap, &header)?;
        let params = ParamTable::build(&directory);

        let mut meshes = MeshTable::build(&directory)?;
        meshes.load_ident_ranges(&a_mmap, header, &directory)?;

        let svars = SvarTable::build(&a_mmap, &directory, header)?;
        let mut srecs = SrecTable::build(&a_mmap, &directory, header)?;
        srecs.patch_m_mesh_classes(&meshes);

        let states = match StateMapSource::pick(&header, &directory) {
            StateMapSource::InlineA(range) => state::parse_inline(&a_mmap, range, &header)?,
            StateMapSource::ExternalTfile => {
                let path = state::tfile_path(&a_path).ok_or(MiliError::MalformedDirectory(
                    "cannot derive tfile path from .A path",
                ))?;
                let bytes = std::fs::read(&path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        MiliError::MalformedDirectory(
                            "directory expects tfile but no <root>T sibling found",
                        )
                    } else {
                        e.into()
                    }
                })?;
                state::parse_tfile(&bytes, &header)?
            }
        };

        Ok(Self {
            a_path,
            a_mmap,
            header,
            directory,
            params,
            meshes,
            svars,
            srecs,
            states,
            state_mmaps: Mutex::new(HashMap::new()),
        })
    }

    /// Re-open this database in place from its `.A` path, replacing
    /// every parsed structure and dropping the cached state mmaps.
    ///
    /// Phase 3.2: after a write that changes the on-disk model
    /// (`append_state` appends a state map, bumps `state_count`, grows
    /// or adds a state file), the in-memory `directory` / `states` /
    /// `a_mmap` are stale. Upstream `_MiliInternal.append_state`
    /// mutates its in-memory `__smaps` so a subsequent `query` /
    /// `state_maps` on the **same** object sees the new state; milox's
    /// engine is the Rust core, so it re-derives that state from the
    /// just-rewritten `.A` (which `append_state` already produces
    /// byte-for-byte vs the upstream `AFileWriter`).
    pub fn reload(&mut self) -> Result<()> {
        *self = Self::open(self.a_path.clone())?;
        Ok(())
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn params(&self) -> &ParamTable {
        &self.params
    }

    /// Per-state metadata in directory order.
    pub fn states(&self) -> &[StateMeta] {
        &self.states
    }

    /// Bytes of the main `.A` file. Byte ranges from
    /// [`crate::directory::DirEntry`] index into this slice.
    pub fn a_bytes(&self) -> &[u8] {
        &self.a_mmap
    }

    /// Path to the `.A` file this database was opened from.
    pub fn a_path(&self) -> &Path {
        &self.a_path
    }

    /// Decode a named scalar / string / array param from the main
    /// directory. Returns `None` if no such named entry exists.
    pub fn param(&self, name: &str) -> Result<Option<ParamValue<'_>>> {
        let Some(idx) = self.params.get(name) else {
            return Ok(None);
        };
        let entry = &self.directory.entries[idx];
        Ok(Some(ParamValue::decode(&self.a_mmap, entry, self.header)?))
    }

    /// Read the `"mesh dimensions"` scalar (`mesh_u.c:296-299, 3900`).
    /// Errors if the param is absent or not an i32 scalar.
    pub fn mesh_dimensions(&self) -> Result<i32> {
        match self.param("mesh dimensions")? {
            Some(ParamValue::Scalar(ScalarValue::I32(n))) => Ok(n),
            Some(_) => Err(MiliError::MalformedDirectory(
                "'mesh dimensions' is not an i32 scalar",
            )),
            None => Err(MiliError::MalformedDirectory(
                "'mesh dimensions' param not found",
            )),
        }
    }

    /// Times in directory order. Convenience over `states().iter().map(|m| m.time)`.
    pub fn times(&self) -> Vec<f32> {
        self.states.iter().map(|m| m.time).collect()
    }

    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    /// Mesh / class metadata folded from the directory's `CLASS_DEF`,
    /// `CLASS_IDENTS`, `NODES`, and `ELEM_CONNS` entries.
    pub fn meshes(&self) -> &MeshTable {
        &self.meshes
    }

    /// State-variable dictionary parsed from every `STATE_VAR_DICT`
    /// entry in the directory.
    pub fn svars(&self) -> &SvarTable {
        &self.svars
    }

    /// State-record schemas parsed from every `STATE_REC_DATA` entry
    /// in the directory.
    pub fn srecs(&self) -> &SrecTable {
        &self.srecs
    }

    /// Decode the `NODES` payload for `(mesh_id, classname)`, returning
    /// `None` if no such entry is registered.
    pub fn nodes(&self, mesh_id: MeshId, classname: &str) -> Result<Option<Nodes<'_>>> {
        let Some(idx) = self.meshes.nodes_entry_index(mesh_id, classname) else {
            return Ok(None);
        };
        let dims = self.mesh_dimensions()?;
        let entry = &self.directory.entries[idx];
        Ok(Some(mesh::decode_nodes(
            &self.a_mmap,
            entry,
            self.header,
            dims,
        )?))
    }

    /// Decode the `ELEM_CONNS` payload for `(mesh_id, classname)`,
    /// returning `None` if no such entry is registered.
    pub fn connectivity(
        &self,
        mesh_id: MeshId,
        classname: &str,
    ) -> Result<Option<Connectivity<'_>>> {
        let Some(idx) = self.meshes.conns_entry_index(mesh_id, classname) else {
            return Ok(None);
        };
        let entry = &self.directory.entries[idx];
        Ok(Some(mesh::decode_elem_conns(
            &self.a_mmap,
            entry,
            self.header,
        )?))
    }

    /// Owned nodal coordinates for `(mesh_id, "node")`, concatenating
    /// **every** `NODES` directory entry in directory order (upstream
    /// iterates `afile.dirs[NODES].items()`,
    /// `miliinternal.py:204-206`). Returns `(flat_row_major, dims)`
    /// where the data is `[node][dim]` and `dims =
    /// mesh_dimensions()`. `None` if the mesh declares no `NODES`
    /// entry.
    ///
    /// This is the owned counterpart to [`Self::nodes`] (which returns
    /// a single borrowed entry view); the binding needs owned data for
    /// the `IntoPyArray` ownership-transfer return.
    pub fn node_coords(&self, mesh_id: MeshId) -> Result<Option<(Vec<f32>, usize)>> {
        let indices = self.meshes.nodes_entry_indices(mesh_id, "node");
        if indices.is_empty() {
            return Ok(None);
        }
        let dims = self.mesh_dimensions()?;
        let dims_us = usize::try_from(dims)
            .map_err(|_| MiliError::MalformedDirectory("mesh dimensions <= 0"))?;
        let mut out: Vec<f32> = Vec::new();
        for &idx in indices {
            let entry = &self.directory.entries[idx];
            let nodes = mesh::decode_nodes(&self.a_mmap, entry, self.header, dims)?;
            out.extend_from_slice(&nodes.to_f32_vec()?);
        }
        Ok(Some((out, dims_us)))
    }

    /// Owned element connectivity as **labels**, matching upstream
    /// `connectivity()` (`miliinternal.py:217-223`,
    /// `miliinternal.py:608`): for each element, the per-disk row is
    /// `[node_1..node_k, material, part]`; the returned row is
    /// `[label(node_1)..label(node_k), material]` — the trailing
    /// `part` column dropped, the `material` column kept verbatim, and
    /// each fortran node id substituted by the node-class label at
    /// position `id - 1` (`node_labels[id - 1]`).
    ///
    /// Returns `(flat_row_major, ncols)` with `ncols = conn_words -
    /// 1`; `None` if the class declares no `ELEM_CONNS` entry. All
    /// `ELEM_CONNS` entries for the class are concatenated in
    /// directory order.
    ///
    /// Distinct from the raw [`Self::connectivity`] /
    /// `Connectivity::to_i32_vec` primitive (which keeps fortran ids +
    /// the part column).
    pub fn connectivity_labels(
        &self,
        mesh_id: MeshId,
        classname: &str,
    ) -> Result<Option<(Vec<i32>, usize)>> {
        let indices = self.meshes.conns_entry_indices(mesh_id, classname);
        if indices.is_empty() {
            return Ok(None);
        }
        let node_labels = self.labels(mesh_id, "node")?.unwrap_or_default();
        let mut out: Vec<i32> = Vec::new();
        let mut ncols = 0usize;
        for &idx in indices {
            let entry = &self.directory.entries[idx];
            let conn = mesh::decode_elem_conns(&self.a_mmap, entry, self.header)?;
            let words = conn.conn_words;
            if words < 2 {
                continue;
            }
            ncols = words - 1;
            let n_nodes = words - 2;
            let raw = conn.to_i32_vec()?;
            for row in raw.chunks_exact(words) {
                for &fid in &row[..n_nodes] {
                    let pos = usize::try_from(fid - 1)
                        .ok()
                        .ok_or(MiliError::MalformedDirectory("connectivity: node id < 1"))?;
                    let label = *node_labels.get(pos).ok_or(MiliError::MalformedDirectory(
                        "connectivity: node id past node-label array",
                    ))?;
                    out.push(label);
                }
                out.push(row[n_nodes]); // material (raw), part column dropped
            }
        }
        Ok(Some((out, ncols)))
    }

    /// Owned element connectivity as zero-based node **ids**, matching
    /// upstream `connectivity_ids()` (`miliinternal.py:213-218`,
    /// `miliinternal.py:631`): `__conns_ids = elem_conn[:,:-1]` (drop
    /// the trailing `part` column, keep the `material` column) then the
    /// node columns are decremented by 1 (`-= 1`) to convert the
    /// fortran 1-based node ids to 0-based indices; the `material`
    /// column is left verbatim.
    ///
    /// Returns `(flat_row_major, ncols)` with `ncols = conn_words - 1`
    /// and the last column the raw material number; `None` if the class
    /// declares no `ELEM_CONNS` entry. All `ELEM_CONNS` entries for the
    /// class are concatenated in directory order (same row order as
    /// [`Self::connectivity_labels`] / [`Self::labels`]).
    pub fn connectivity_ids(
        &self,
        mesh_id: MeshId,
        classname: &str,
    ) -> Result<Option<(Vec<i32>, usize)>> {
        let indices = self.meshes.conns_entry_indices(mesh_id, classname);
        if indices.is_empty() {
            return Ok(None);
        }
        let mut out: Vec<i32> = Vec::new();
        let mut ncols = 0usize;
        for &idx in indices {
            let entry = &self.directory.entries[idx];
            let conn = mesh::decode_elem_conns(&self.a_mmap, entry, self.header)?;
            let words = conn.conn_words;
            if words < 2 {
                continue;
            }
            ncols = words - 1;
            let n_nodes = words - 2;
            let raw = conn.to_i32_vec()?;
            for row in raw.chunks_exact(words) {
                for &fid in &row[..n_nodes] {
                    out.push(fid - 1); // fortran 1-based -> 0-based index
                }
                out.push(row[n_nodes]); // material (raw), part column dropped
            }
        }
        Ok(Some((out, ncols)))
    }

    /// Concatenated label array for `(mesh_id, classname)`.
    ///
    /// Implements the TI_PARAM-as-storage recipe from
    /// `planning/shared/format.md` § "TI_PARAM-as-storage pattern":
    /// for the `"node"` class, scan TI_PARAMs whose name starts with
    /// `"Node Labels"`; for any other class, scan names starting with
    /// `"Element Labels"`, skipping any that contain `"ElemIds"`. Filter
    /// to those whose descriptor's `Sname-<classname>` matches, then
    /// concatenate every matching payload in directory order. Returns
    /// `None` if no matching entry is present.
    ///
    /// `reference/mili-python/src/mili/miliinternal.py:96-106`.
    pub fn labels(&self, mesh_id: MeshId, classname: &str) -> Result<Option<Vec<i32>>> {
        let prefix = if classname == "node" {
            "Node Labels"
        } else {
            "Element Labels"
        };
        let sname_token = format!("Sname-{classname}");
        let mut out: Vec<i32> = Vec::new();
        let mut any = false;
        for entry in &self.directory.entries {
            if entry.entry_type != DirEntryType::TiParam || entry.name_count == 0 {
                continue;
            }
            let name = self.directory.names.get(entry.name_start as usize);
            if !name.starts_with(prefix) || name.contains("ElemIds") {
                continue;
            }
            if !descriptor_matches(name, mesh_id, &sname_token) {
                continue;
            }
            any = true;
            append_i32_array(&self.a_mmap, entry, self.header, &mut out)?;
        }
        if any {
            return Ok(Some(out));
        }
        // No TI "Element/Node Labels" param: fall back to the class's
        // `CLASS_IDENTS` ranges expanded to `[start..=stop]`, mirroring
        // upstream `miliinternal.py:198-202` (which seeds `__labels`
        // for every `CLASS_IDENTS` class). Element classes hit the TI
        // path above; this covers ident-only classes (mat / glob /
        // lcurve / mesh).
        if let Some(class) = self.meshes.mesh(mesh_id).and_then(|m| m.class(classname)) {
            if !class.id_blocks.is_empty() {
                for &(start, stop) in &class.id_blocks {
                    out.extend(start..=stop);
                }
                return Ok(Some(out));
            }
        }
        Ok(None)
    }

    /// Whether `classname` had a real *ident* source — a TI
    /// `Node Labels` / `Element Labels` entry or a `CLASS_IDENTS`
    /// entry — i.e. whether it would be in upstream's pre-finalisation
    /// `__labels` (`miliinternal.py:175-202`). Deliberately ignores the
    /// `NODES` / `ELEM_CONNS` id-range fallback that
    /// [`Self::labels`] folds in, because upstream's
    /// `MeshObjectClass.idents_exist` is `False` exactly when the class
    /// reaches finalisation *without* such a source
    /// (`miliinternal.py:276-282`).
    pub fn idents_exist(&self, mesh_id: MeshId, classname: &str) -> Result<bool> {
        let prefix = if classname == "node" {
            "Node Labels"
        } else {
            "Element Labels"
        };
        let sname_token = format!("Sname-{classname}");
        for entry in &self.directory.entries {
            match entry.entry_type {
                DirEntryType::TiParam if entry.name_count > 0 => {
                    let name = self.directory.names.get(entry.name_start as usize);
                    if name.starts_with(prefix)
                        && !name.contains("ElemIds")
                        && descriptor_matches(name, mesh_id, &sname_token)
                    {
                        return Ok(true);
                    }
                }
                DirEntryType::ClassIdents if entry.name_count > 0 => {
                    let (eid, ecls) =
                        crate::mesh::mesh_and_classname(entry, &self.directory.names)?;
                    if eid == mesh_id && ecls == classname {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Materials discovered via `MAT_NAME_<n>` TI_PARAM entries.
    ///
    /// Returns a map from material name (the entry's string payload) to
    /// the list of material numbers (the `<n>` suffix) that share that
    /// name. Material numbers appear in the order their TI_PARAMs are
    /// encountered.
    ///
    /// `reference/mili-python/src/mili/miliinternal.py:198-211`.
    pub fn materials(&self) -> Result<HashMap<String, Vec<i32>>> {
        let mut out: HashMap<String, Vec<i32>> = HashMap::new();
        for entry in &self.directory.entries {
            if entry.entry_type != DirEntryType::TiParam || entry.name_count == 0 {
                continue;
            }
            let name = self.directory.names.get(entry.name_start as usize);
            let Some(suffix) = name.strip_prefix("MAT_NAME_") else {
                continue;
            };
            let Ok(n) = suffix.parse::<i32>() else {
                continue;
            };
            let value = ParamValue::decode(&self.a_mmap, entry, self.header)?;
            let ParamValue::String(s) = value else {
                return Err(MiliError::MalformedDirectory(
                    "MAT_NAME_<n>: payload not a string param",
                ));
            };
            out.entry(s.to_owned()).or_default().push(n);
        }
        Ok(out)
    }

    /// Element sets defined via `IntLabel_es_<setname>` TI_PARAMs.
    ///
    /// The returned `Vec<i32>` is the raw payload exactly as written:
    /// integration-point ids followed by a single trailing count entry.
    /// See `planning/shared/format.md` § "TI_PARAM-as-storage pattern"
    /// and `reference/mili-python/src/mili/miliinternal.py:113-115`.
    pub fn element_sets(&self) -> Result<HashMap<String, Vec<i32>>> {
        let mut out: HashMap<String, Vec<i32>> = HashMap::new();
        for entry in &self.directory.entries {
            if entry.entry_type != DirEntryType::TiParam || entry.name_count == 0 {
                continue;
            }
            let name = self.directory.names.get(entry.name_start as usize);
            // Upstream keys by `sname[sname.find('es_'):]`
            // (`miliinternal.py:113-115`) — i.e. the name from `es_`
            // onward, e.g. `IntLabel_es_5` → `es_5`. Strip only the
            // `IntLabel_` prefix.
            let Some(setname) = name.strip_prefix("IntLabel_") else {
                continue;
            };
            if !setname.starts_with("es_") {
                continue;
            }
            let mut values = Vec::new();
            append_i32_array(&self.a_mmap, entry, self.header, &mut values)?;
            out.insert(setname.to_owned(), values);
        }
        Ok(out)
    }

    /// Build the svar→element-set→IP-label linkage (the core analogue of
    /// upstream `_MiliInternal.__int_points`,
    /// `reference/mili-python/src/mili/miliinternal.py:156-192`). A
    /// VEC_ARRAY svar `es_<n>a` whose name minus its last char is an
    /// element-set key links every component it carries (recursively)
    /// to that set's IP-label payload, plus the upstream `stress` /
    /// `strain` special-case and the query-by-set-name self entry.
    /// Empty when the family declares no element sets (the common
    /// scalar/vector corpus, and every multi-fragment basic1 part).
    pub(crate) fn build_int_points(&self) -> IntPoints {
        let mut ip = IntPoints::default();
        let Ok(element_sets) = self.element_sets() else {
            return ip;
        };
        if element_sets.is_empty() {
            return ip;
        }
        let direct_comps = |name: &str| -> Vec<String> {
            match self.svars.get(name).map(|s| &s.agg) {
                Some(SvarAgg::Vector { comps } | SvarAgg::VecArray { comps, .. }) => comps.clone(),
                _ => Vec::new(),
            }
        };
        let stress_comps = direct_comps("stress");
        let strain_comps = direct_comps("strain");
        for sv in self.svars.iter() {
            let Some(eset_name) = sv.name.get(..sv.name.len().saturating_sub(1)) else {
                continue;
            };
            let Some(payload) = element_sets.get(eset_name) else {
                continue;
            };
            let comps = direct_comps(&sv.name);
            // Upstream `stress` / `strain` special blocks: an element
            // set whose direct components *are* the six stress (resp.
            // strain) components is also queryable by `"stress"` /
            // `"strain"` (`miliinternal.py:170-181`).
            if stress_comps.len() == 6
                && comps.iter().filter(|c| stress_comps.contains(c)).count() == 6
            {
                ip.insert("stress", &sv.name, payload);
            }
            if strain_comps.len() == 6
                && comps.iter().filter(|c| strain_comps.contains(c)).count() == 6
            {
                ip.insert("strain", &sv.name, payload);
            }
            self.add_int_points(&mut ip, &sv.name, &comps, payload);
            // Query-by-element-set-name self entry (`miliinternal.py:191`).
            ip.insert(&sv.name, &sv.name, payload);
        }
        ip
    }

    /// Recursive half of [`Self::build_int_points`] — upstream
    /// `addIntPoints` (`miliinternal.py:156-163`): link every
    /// (transitive) component of the element-set VEC_ARRAY to its
    /// IP-label payload.
    fn add_int_points(&self, ip: &mut IntPoints, es_name: &str, comps: &[String], payload: &[i32]) {
        for c in comps {
            ip.insert(c, es_name, payload);
            if let Some(SvarAgg::Vector { comps: cc } | SvarAgg::VecArray { comps: cc, .. }) =
                self.svars.get(c).map(|s| s.agg.clone())
            {
                self.add_int_points(ip, es_name, &cc, payload);
            }
        }
    }

    /// Read every value of `svar_name` on objects of class `class` at
    /// state index `state_idx`, returning a typed flat vector keyed by
    /// the svar's numeric type.
    ///
    /// Equivalent to a [`Self::query`] with no filters and a single
    /// state. Flat output layout is `[object][atom]` row-major over the
    /// concatenated set of matching subrecs in directory order; per-
    /// object atom count is `Svar::atoms`.
    pub fn state_var_values(
        &self,
        svar_name: &str,
        class: &str,
        state_idx: usize,
    ) -> Result<StateValues> {
        let states = [state_idx];
        self.query(&QueryArgs {
            svar: svar_name,
            class,
            labels: None,
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
    }

    /// Run a filtered, multi-state query for one svar against one class.
    ///
    /// Output layout is flat `[state][label][atom]` row-major. With no
    /// filters, the `label` axis is every object of every matching
    /// subrec in directory order; with [`QueryArgs::labels`] set, it is
    /// the labels in argument order. With [`QueryArgs::ips`] set the
    /// `atom` axis is `comps * ips.len()` instead of `Svar::atoms`.
    ///
    /// Materials are translated to a label list through this database's
    /// `ELEM_CONNS` payload (`mesh_u.c:1556-1678` write side,
    /// `miliinternal.py:225-228` read side). A material that selects
    /// the same label twice via overlapping element-conns is collapsed
    /// to a single occurrence, but the relative order of labels in
    /// connectivity is preserved.
    pub fn query(&self, args: &QueryArgs<'_>) -> Result<StateValues> {
        self.query_with_labels(args).map(|(v, _)| v)
    }

    /// Run a filtered, multi-state query and return both the values and
    /// the entity-axis labels (in `[label]` order matching the
    /// `[state][label][atom]` layout of the returned `StateValues`).
    ///
    /// This is the primitive used by [`crate::family_set::DatabaseSet`]
    /// when merging per-fragment results. End users should prefer
    /// [`Database::query`] plus [`Database::labels`].
    pub fn query_with_labels(&self, args: &QueryArgs<'_>) -> Result<(StateValues, Vec<i32>)> {
        let (values, labels, _) = self.run_query(args)?;
        Ok((values, labels))
    }

    /// Core query: values, entity-axis labels, and an optional
    /// component-name override (the Slice-B VEC_ARRAY-substitution path
    /// resolves `f"{comp} ipt. {label}"` names during planning).
    // Material→label resolution, label→mo-id mapping, the multi-state
    // gather macro, and entity-axis remap are sequential and
    // individually small; splitting them would only scatter the
    // single linear query flow.
    #[allow(clippy::too_many_lines)]
    fn run_query(
        &self,
        args: &QueryArgs<'_>,
    ) -> Result<(StateValues, Vec<i32>, Option<Vec<String>>)> {
        if args.states.is_empty() {
            return Err(MiliError::MalformedDirectory(
                "query: states must be non-empty",
            ));
        }
        for &s in args.states {
            if s >= self.states.len() {
                return Err(MiliError::StateOutOfRange(s, self.states.len()));
            }
        }

        // Resolve material filter into a label list. If both materials
        // and labels are provided, take their intersection in
        // `args.labels` order — mili-python's `query` accepts either
        // form but never both for the same call; we accept both for
        // forward compatibility.
        let material_labels: Option<Vec<i32>> = match args.materials {
            Some(mats) => Some(self.labels_for_materials(args.class, mats)?),
            None => None,
        };
        let resolved_labels: Option<Vec<i32>> = match (args.labels, material_labels.as_deref()) {
            (None, None) => None,
            (None, Some(m)) => Some(m.to_vec()),
            (Some(l), None) => Some(l.to_vec()),
            (Some(l), Some(m)) => Some(l.iter().copied().filter(|x| m.contains(x)).collect()),
        };

        // Map the requested entity labels into the on-disk subrecord
        // ordinal space (1-based class mesh-object ids), mirroring
        // upstream `np.where(np.isin(labels_of_class, labels))[0]`
        // (`reference/mili-python/src/mili/miliinternal.py:1183`). The
        // subrecord `id_blocks` enumerate mo ids, not user labels —
        // they coincide only when a class has contiguous `1..=qty`
        // labels, which is why this divergence stayed hidden until the
        // dbl_nodtang (diablo) corpus, whose `cbs1_particle` labels are
        // `[5,10,..,125]` over mo ids `1..=25`.
        let mo_id_labels: Option<Vec<i32>> = match &resolved_labels {
            Some(l) => {
                let ids = self.labels_to_mo_ids(args.class, l)?;
                // Upstream errors when the label/material filter
                // resolves to no class ordinal at all
                // (`miliinternal.py:1196-1198`,
                // `ReturnCode.ERROR "No labels found for the class"`);
                // a *partial* match (some labels absent) is not an
                // error — `np.isin` just drops the missing ones.
                if ids.is_empty() {
                    return Err(MiliError::LabelNotFound {
                        label: l.first().copied().unwrap_or_default(),
                        class: args.class.to_owned(),
                    });
                }
                Some(ids)
            }
            None => None,
        };

        let filter = Filter {
            labels: mo_id_labels.as_deref(),
            ips: args.ips,
            subrec: args.subrec,
        };

        // Build a plan against the first state, then rebase per state.
        // All requested states must share an srec format — mixed srec
        // formats break the precomputable-offsets invariant
        // (`reference/mili-python/src/mili/miliinternal.py:1393`).
        let first = self.states[args.states[0]];
        let first_srec_format = first.srec_format;
        for &s in &args.states[1..] {
            if self.states[s].srec_format != first_srec_format {
                return Err(MiliError::Unsupported(
                    "query across states with differing srec formats",
                ));
            }
        }
        let srec = self
            .srecs
            .get(first_srec_format)
            .ok_or(MiliError::MalformedDirectory(
                "state references unknown srec_format",
            ))?;
        let first_data_start = state_data_start(first)?;
        let int_points = self.build_int_points();
        let plan = plan_state_svar_ip(
            srec,
            &self.svars,
            args.svar,
            args.class,
            first_data_start,
            filter,
            &int_points,
        )?;
        let plan_components = plan.components.clone();

        let byteswap = !self.header.is_native_endian();
        let bytes_per_state = plan.total_bytes();
        let total_bytes =
            bytes_per_state
                .checked_mul(args.states.len())
                .ok_or(MiliError::MalformedDirectory(
                    "query: total byte count overflow",
                ))?;
        let width = plan.num_type.width();
        let count = total_bytes / width;
        let count_per_state = bytes_per_state / width;

        // Per-state context: rebased plan + mmap + path. Resolving the
        // mmap touches `state_mmaps: Mutex<HashMap>` so do it once up
        // front rather than inside the parallel iterator. The rebase
        // also touches a checked-arithmetic path that can fail; keep
        // it serial so errors short-circuit cleanly.
        let ctxs: Vec<StateCtx> = self.build_state_contexts(args.states, &plan)?;

        macro_rules! gather {
            ($ty:ty, $variant:ident) => {{
                let mut v: Vec<$ty> = vec![<$ty>::default(); count];
                v.par_chunks_mut(count_per_state)
                    .zip(ctxs.par_iter())
                    .try_for_each(|(chunk, ctx)| -> Result<()> {
                        let mut out_idx = 0usize;
                        for slab in &ctx.plan.slabs {
                            let b = slab_bytes(&ctx.mmap, &ctx.path, slab.start, slab.len)?;
                            crate::endian::for_each_swap::<$ty, _>(b, byteswap, |x| {
                                chunk[out_idx] = x;
                                out_idx += 1;
                            });
                        }
                        Ok(())
                    })?;
                StateValues::$variant(v)
            }};
        }

        let values = match plan.num_type {
            NumType::Float4 => gather!(f32, F32),
            NumType::Float8 => gather!(f64, F64),
            NumType::Int4 => gather!(i32, I32),
            NumType::Int8 => gather!(i64, I64),
        };

        // Both the filtered (`gather_by_labels`) and unfiltered
        // (`gather_all`) paths now emit subrecord mesh-object ids — the
        // filter labels were pre-mapped into the mo-id space above — so
        // the entity axis is always mapped back through the class label
        // array (see `map_mo_ids_to_labels`).
        let labels = self.map_mo_ids_to_labels(args.class, plan.labels)?;
        Ok((values, labels, plan_components))
    }

    /// QueryDict-shaped result (`planning/mili-py/m3.md`). Reuses
    /// [`Self::query_with_labels`] for the value/label axes and derives
    /// the parity-sensitive `components` / `title` from the svar table
    /// (`reference/mili-python/src/mili/miliinternal.py:1320-1336`).
    /// Primal only; `states`/`times` are attached caller-side.
    pub fn query_full(&self, args: &QueryArgs<'_>) -> Result<QueryResult> {
        let (values, labels, comp_override) = self.run_query(args)?;
        let (fallback_components, title) = self.svar_query_meta(args.svar)?;
        let components = comp_override.unwrap_or(fallback_components);
        Ok(QueryResult {
            values,
            labels,
            components,
            title,
            class_name: args.class.to_owned(),
        })
    }

    /// Phase 3.2 (decision 23): the `query(write_data=)` write-half —
    /// the **inverse** of [`Self::run_query`]'s byte gather. It builds
    /// the *same* per-state [`ReadPlan`] a read of
    /// `(svar, class, labels, states, ips, subrec)` would, then
    /// scatters `write_data` into exactly those byte slabs in the
    /// state files (`rb+`, each state at its smap data-start),
    /// reproducing upstream `_MiliInternal.__query`'s
    /// `srec.extract_ordinals(write_data=)` path
    /// (`reference/mili-python/src/mili/miliinternal.py:1322-1416`):
    /// `var_data[ordinals] = write_data` then write the buffer back.
    ///
    /// `wd_labels` is `write_data[svar]['layout']['labels']`; `values`
    /// is `write_data[svar]['data']` flattened C-order
    /// `[state][label][atom]`, its state axis **positionally** aligned
    /// with `args.states` (upstream indexes it by the requested-state
    /// position `sidx`, not a state-label lookup —
    /// `miliinternal.py:1396/1409`). Per-label rows are realigned from
    /// the `wd_labels` order to the result order via the same lookup
    /// upstream spells `argsort`/`searchsorted`
    /// (`miliinternal.py:1331-1334`). `values` is `f64`; numpy casts
    /// `write_data` to the svar dtype on the `var_data[...] =`
    /// assignment, so the cast to the plan's [`NumType`] reproduces
    /// that bit-for-bit for the corpus (f32/f64/i32 svars).
    pub fn scatter_query(
        &self,
        args: &QueryArgs<'_>,
        wd_labels: &[i32],
        values: &[f64],
    ) -> Result<()> {
        if args.states.is_empty() {
            return Err(MiliError::MalformedDirectory(
                "scatter: states must be non-empty",
            ));
        }
        for &s in args.states {
            if s >= self.states.len() {
                return Err(MiliError::StateOutOfRange(s, self.states.len()));
            }
        }

        // Filter resolution mirrors `run_query` exactly (the scatter
        // must hit the identical byte slabs the read gather would).
        let material_labels: Option<Vec<i32>> = match args.materials {
            Some(mats) => Some(self.labels_for_materials(args.class, mats)?),
            None => None,
        };
        let resolved_labels: Option<Vec<i32>> = match (args.labels, material_labels.as_deref()) {
            (None, None) => None,
            (None, Some(m)) => Some(m.to_vec()),
            (Some(l), None) => Some(l.to_vec()),
            (Some(l), Some(m)) => Some(l.iter().copied().filter(|x| m.contains(x)).collect()),
        };
        let mo_id_labels: Option<Vec<i32>> = match &resolved_labels {
            Some(l) => {
                let ids = self.labels_to_mo_ids(args.class, l)?;
                if ids.is_empty() {
                    return Err(MiliError::LabelNotFound {
                        label: l.first().copied().unwrap_or_default(),
                        class: args.class.to_owned(),
                    });
                }
                Some(ids)
            }
            None => None,
        };
        let filter = Filter {
            labels: mo_id_labels.as_deref(),
            ips: args.ips,
            subrec: args.subrec,
        };

        let first = self.states[args.states[0]];
        let first_srec_format = first.srec_format;
        for &s in &args.states[1..] {
            if self.states[s].srec_format != first_srec_format {
                return Err(MiliError::Unsupported(
                    "scatter across states with differing srec formats",
                ));
            }
        }
        let srec = self
            .srecs
            .get(first_srec_format)
            .ok_or(MiliError::MalformedDirectory(
                "state references unknown srec_format",
            ))?;
        let first_data_start = state_data_start(first)?;
        let int_points = self.build_int_points();
        let plan = plan_state_svar_ip(
            srec,
            &self.svars,
            args.svar,
            args.class,
            first_data_start,
            filter,
            &int_points,
        )?;

        let result_labels = self.map_mo_ids_to_labels(args.class, plan.labels.clone())?;
        let n_lab = result_labels.len();
        let width = plan.num_type.width();
        let total_per_state = plan.total_bytes();
        if n_lab == 0 || total_per_state == 0 {
            return Ok(());
        }
        let atoms_total = total_per_state / width;
        if atoms_total % n_lab != 0 {
            return Err(MiliError::MalformedDirectory(
                "scatter: gather plan size not divisible by the label count",
            ));
        }
        let atoms = atoms_total / n_lab;

        // `wd_labels` order → row index, mirroring upstream's
        // `argsort`/`searchsorted` realignment to the result order
        // (`miliinternal.py:1331-1334`). Labels are unique per query.
        let n_wd = wd_labels.len();
        let mut row_of: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
        for (i, &l) in wd_labels.iter().enumerate() {
            row_of.entry(l).or_insert(i);
        }
        let rows: Vec<usize> = result_labels
            .iter()
            .map(|l| {
                row_of.get(l).copied().ok_or(MiliError::MalformedDirectory(
                    "scatter: a result label is absent from write_data['layout']['labels']",
                ))
            })
            .collect::<Result<_>>()?;

        let n_states = args.states.len();
        let expected = n_states
            .checked_mul(n_wd)
            .and_then(|x| x.checked_mul(atoms))
            .ok_or(MiliError::MalformedDirectory("scatter: size overflow"))?;
        if values.len() != expected {
            return Err(MiliError::MalformedDirectory(
                "scatter: write_data['data'] length does not match \
                 states * labels * atoms",
            ));
        }

        let end = self.header.endianness;
        let encode = |v: f64, out: &mut Vec<u8>| match plan.num_type {
            NumType::Float4 => {
                let b = v as f32;
                match end {
                    Endianness::Big => out.extend_from_slice(&b.to_be_bytes()),
                    Endianness::Little => out.extend_from_slice(&b.to_le_bytes()),
                }
            }
            NumType::Float8 => match end {
                Endianness::Big => out.extend_from_slice(&v.to_be_bytes()),
                Endianness::Little => out.extend_from_slice(&v.to_le_bytes()),
            },
            NumType::Int4 => {
                let b = v as i32;
                match end {
                    Endianness::Big => out.extend_from_slice(&b.to_be_bytes()),
                    Endianness::Little => out.extend_from_slice(&b.to_le_bytes()),
                }
            }
            NumType::Int8 => {
                let b = v as i64;
                match end {
                    Endianness::Big => out.extend_from_slice(&b.to_be_bytes()),
                    Endianness::Little => out.extend_from_slice(&b.to_le_bytes()),
                }
            }
        };

        for (p, &sidx) in args.states.iter().enumerate() {
            let state = self.states[sidx];
            let data_start = state_data_start(state)?;
            let rebased = plan.rebased(data_start)?;
            let path = state_file_path(&self.a_path, self.header.suffix_width, state.file).ok_or(
                MiliError::MalformedDirectory("cannot derive state-file path from .A path"),
            )?;

            // Per-state payload in `[label][atom]` slab order.
            let mut buf: Vec<u8> = Vec::with_capacity(total_per_state);
            for &row in &rows {
                for j in 0..atoms {
                    let idx = (p * n_wd + row) * atoms + j;
                    encode(values[idx], &mut buf);
                }
            }

            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)?;
            let mut cursor = 0usize;
            for slab in &rebased.slabs {
                use std::io::{Seek, SeekFrom, Write};
                f.seek(SeekFrom::Start(slab.start as u64))?;
                f.write_all(&buf[cursor..cursor + slab.len])?;
                cursor += slab.len;
            }
        }

        // The read path mmaps state files MAP_SHARED read-only; a
        // pre-existing mapping reflects these `write(2)`s via the page
        // cache, but drop the cache so a re-read is unambiguous.
        self.state_mmaps
            .lock()
            .expect("state_mmaps mutex not poisoned")
            .clear();

        Ok(())
    }

    /// Component-name list + title for the queried svar, mirroring
    /// upstream's `svars_to_query` component expansion
    /// (`reference/mili-python/src/mili/miliinternal.py:1362-1378`):
    /// scalar → `[name]`; array → `name[1..=dims0]`; **array with an
    /// explicit subscript `name[i,j]` → the single combined
    /// `name[i,j]`** (upstream line 1371); vector / vec-array → its
    /// component svars expanded recursively. The ip-filtered
    /// `f"{comp} ipt. {label}"` form is handled by the caller-supplied
    /// `ips` path below.
    pub(crate) fn svar_query_meta(&self, svar_name: &str) -> Result<(Vec<String>, String)> {
        let base = svar_name.split('[').next().unwrap_or(svar_name);
        let svar = self
            .svars
            .get(base)
            .ok_or_else(|| MiliError::UnknownSvar(base.to_owned()))?;
        let mut comps = Vec::new();
        // Subscript form (`hx[3]`, `g[1,2]`): one combined component
        // name carrying the original 1-based indices, not the full
        // `hx[1..=dims0]` expansion. Mirrors upstream line 1371.
        match crate::query::parse_query_name(svar_name)? {
            crate::query::QueryName::Subscript { base: b, indices } => {
                let joined = indices
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                comps.push(format!("{b}[{joined}]"));
                return Ok((comps, svar.title.clone()));
            }
            // Named-component subscript `parent[comp]`: the component
            // names are the fallback `components`, the title is the
            // *parent's* (`miliinternal.py:1217` keys title by the
            // resolved svar = the parent). The ip-filtered
            // `f"{comp} ipt. {label}"` form, when present, arrives as
            // the caller's `comp_override` and wins.
            crate::query::QueryName::CompSubscript { comps: cs, .. } => {
                for c in cs {
                    comps.push(c.to_owned());
                }
                return Ok((comps, svar.title.clone()));
            }
            crate::query::QueryName::Plain(_) => {}
        }
        self.expand_components(base, &svar.agg, &mut comps);
        Ok((comps, svar.title.clone()))
    }

    fn expand_components(&self, name: &str, agg: &SvarAgg, out: &mut Vec<String>) {
        match agg {
            SvarAgg::Scalar => out.push(name.to_owned()),
            SvarAgg::Array { dims } => {
                let n = dims.first().copied().unwrap_or(0);
                for i in 1..=n {
                    out.push(format!("{name}[{i}]"));
                }
            }
            SvarAgg::Vector { comps } | SvarAgg::VecArray { comps, .. } => {
                for c in comps {
                    match self.svars.get(c) {
                        Some(cs) => self.expand_components(c, &cs.agg, out),
                        None => out.push(c.clone()),
                    }
                }
            }
        }
    }

    /// Map the unfiltered-query entity axis from subrecord mesh-object
    /// ids (1-based ordinals into the class) to user-facing entity
    /// labels, mirroring mili-python's `miliinternal.py:1297`
    /// (`class_labels[ordinals_in_srec]`).
    ///
    /// Single-fragment callers discard the label vector so the
    /// distinction was historically invisible, but
    /// [`crate::family_set::DatabaseSet`] merges and dedupes on it and
    /// raw ordinals collide across fragments. When the class has no
    /// `Labels` TI param mili-python defaults labels to the MO ids
    /// themselves (`miliinternal.py:281`), so the ordinal vector is
    /// already correct and is returned unchanged.
    /// Map requested entity labels onto 1-based class mesh-object ids
    /// (the on-disk subrecord `id_blocks` ordinal space), mirroring
    /// upstream `np.where(np.isin(labels_of_class, labels))[0]`
    /// (`reference/mili-python/src/mili/miliinternal.py:1183`):
    /// positions are taken in class-label-array order (ascending) and a
    /// requested label absent from the class is silently dropped.
    /// Returns the labels unchanged when the class has no explicit
    /// label array — upstream then defaults its labels to
    /// `arange(1, qty+1)` (`miliinternal.py:281`), so a label already
    /// equals its mo id.
    fn labels_to_mo_ids(&self, class: &str, labels: &[i32]) -> Result<Vec<i32>> {
        let Some(class_labels) = self.labels(MeshId(0), class)? else {
            return Ok(labels.to_vec());
        };
        let want: std::collections::HashSet<i32> = labels.iter().copied().collect();
        let mut mo_ids = Vec::with_capacity(labels.len().min(class_labels.len()));
        for (idx, &lbl) in class_labels.iter().enumerate() {
            if want.contains(&lbl) {
                mo_ids.push(idx as i32 + 1);
            }
        }
        Ok(mo_ids)
    }

    fn map_mo_ids_to_labels(&self, class: &str, mo_ids: Vec<i32>) -> Result<Vec<i32>> {
        let Some(class_labels) = self.labels(MeshId(0), class)? else {
            return Ok(mo_ids);
        };
        let mut mapped = Vec::with_capacity(mo_ids.len());
        for &mo_id in &mo_ids {
            let real = usize::try_from((mo_id as i64) - 1)
                .ok()
                .and_then(|i| class_labels.get(i).copied())
                .ok_or(MiliError::MalformedDirectory(
                    "query: subrecord mesh-object id outside class label range",
                ))?;
            mapped.push(real);
        }
        Ok(mapped)
    }

    /// Prefetch the per-state read context (rebased plan, state-file
    /// mmap, state-file path) for every requested state. Touches
    /// `state_mmaps` and the rebase math single-threaded so the
    /// parallel gather pass sees only `&self` and disjoint output
    /// chunks.
    fn build_state_contexts(&self, states: &[usize], plan: &ReadPlan) -> Result<Vec<StateCtx>> {
        let mut ctxs = Vec::with_capacity(states.len());
        for (i, &sidx) in states.iter().enumerate() {
            let state = self.states[sidx];
            let mmap = self.state_mmap(state.file)?;
            let path = state_file_path(&self.a_path, self.header.suffix_width, state.file).ok_or(
                MiliError::MalformedDirectory("cannot derive state-file path from .A path"),
            )?;
            let rebased = if i == 0 {
                plan.clone()
            } else {
                plan.rebased(state_data_start(state)?)?
            };
            ctxs.push(StateCtx {
                plan: rebased,
                mmap,
                path,
            });
        }
        Ok(ctxs)
    }

    /// Walk `ELEM_CONNS` for `classname` and return the labels whose
    /// `mat_id` column matches any value in `materials`. Order follows
    /// connectivity-row order across all matching `ELEM_CONNS` entries;
    /// each label appears at most once.
    fn labels_for_materials(&self, classname: &str, materials: &[i32]) -> Result<Vec<i32>> {
        if materials.is_empty() {
            return Ok(Vec::new());
        }
        let mesh_id = MeshId(0);
        let mesh = self
            .meshes
            .mesh(mesh_id)
            .ok_or(MiliError::UnknownClass(classname.to_owned()))?;
        let class = mesh
            .class(classname)
            .ok_or_else(|| MiliError::UnknownClass(classname.to_owned()))?;
        let conn_idxs = self.meshes.conns_entry_indices(mesh_id, classname);
        if conn_idxs.is_empty() {
            return Err(MiliError::UnknownMaterial {
                material: materials[0],
            });
        }

        let class_labels = self
            .labels(mesh_id, classname)?
            .unwrap_or_else(|| label_range(&class.id_blocks));
        let mut keep_indices: Vec<usize> = Vec::new();
        let mut row_offset: usize = 0;
        let mut seen_materials: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for &idx in conn_idxs {
            let entry = &self.directory.entries[idx];
            let conn = mesh::decode_elem_conns(&self.a_mmap, entry, self.header)?;
            let words = conn.conn_words;
            if words < 2 {
                return Err(MiliError::MalformedDirectory(
                    "material filter: connectivity has no mat_id column",
                ));
            }
            let row_count = conn.data.len() / (words * 4);
            let raw = conn.to_i32_vec()?;
            // mat_id is the second-to-last column; the last is part_id.
            for row in 0..row_count {
                let mat = raw[row * words + words - 2];
                if materials.contains(&mat) {
                    seen_materials.insert(mat);
                    keep_indices.push(row_offset + row);
                }
            }
            row_offset += row_count;
        }
        for &m in materials {
            if !seen_materials.contains(&m) {
                return Err(MiliError::UnknownMaterial { material: m });
            }
        }

        let mut out = Vec::with_capacity(keep_indices.len());
        for idx in keep_indices {
            if let Some(&label) = class_labels.get(idx) {
                out.push(label);
            }
        }
        Ok(out)
    }

    fn state_mmap(&self, file_idx: i32) -> Result<Arc<Mmap>> {
        let mut map = self
            .state_mmaps
            .lock()
            .expect("state_mmaps mutex not poisoned");
        if let Some(m) = map.get(&file_idx) {
            return Ok(Arc::clone(m));
        }
        let path = state_file_path(&self.a_path, self.header.suffix_width, file_idx).ok_or(
            MiliError::MalformedDirectory("cannot derive state-file path from .A path"),
        )?;
        let file = File::open(&path)?;
        // SAFETY: same posture as the .A mmap — the family is not
        // concurrent-write safe and the database holds the file open
        // through its lifetime.
        let mmap = unsafe { Mmap::map(&file)? };
        let arc = Arc::new(mmap);
        map.insert(file_idx, Arc::clone(&arc));
        Ok(arc)
    }

    /// Integration-point ids per material, derived from
    /// [`Self::element_sets`].
    ///
    /// The material id is the **last character** of the element-set
    /// name (`mat = eset[-1:]` upstream), and the IP ids are the
    /// payload minus the trailing count entry. Sets whose last
    /// character is not a digit are skipped.
    ///
    /// `reference/mili-python/src/mili/miliinternal.py:463-474`.
    pub fn integration_points(&self) -> Result<HashMap<MaterialId, Vec<i32>>> {
        let sets = self.element_sets()?;
        let mut out: HashMap<MaterialId, Vec<i32>> = HashMap::new();
        for (setname, values) in sets {
            let Ok(mat) = setname[setname.len() - 1..].parse::<i32>() else {
                continue;
            };
            let ips = match values.split_last() {
                Some((_count, head)) => head.to_vec(),
                None => Vec::new(),
            };
            out.insert(MaterialId(mat), ips);
        }
        Ok(out)
    }

    /// Element class short names for `mesh_id`, in declaration order.
    ///
    /// Mirrors upstream `class_names()` (`list(__MO_class_data.keys())`,
    /// `miliinternal.py:409-415`) — the order classes are declared by
    /// `CLASS_DEF`, preserved by `Mesh::class_order`. Empty if the mesh
    /// has no classes (or is absent). Signature is symmetric with
    /// [`DatabaseSet::class_names`](crate::DatabaseSet::class_names).
    pub fn class_names(&self, mesh_id: MeshId) -> Vec<String> {
        self.meshes
            .mesh(mesh_id)
            .map(|m| m.class_names().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// Distinct material numbers that own elements, in first-occurrence
    /// order across the connectivity stream.
    ///
    /// Mirrors upstream `material_numbers()`
    /// (`np.array(list(__elems_of_mat.keys()))`,
    /// `miliinternal.py:595-601`): iterate `ELEM_CONNS` entries in
    /// directory order, take the per-entry material column
    /// (`conn[:, -2]`) sorted-unique (`np.unique` is ascending), and
    /// dedupe across entries keeping the first occurrence (upstream's
    /// `defaultdict` key order).
    pub fn material_numbers(&self) -> Result<Vec<i32>> {
        let mut seen: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut out: Vec<i32> = Vec::new();
        for entry in &self.directory.entries {
            if entry.entry_type != DirEntryType::ElemConns {
                continue;
            }
            let conn = mesh::decode_elem_conns(&self.a_mmap, entry, self.header)?;
            if conn.conn_words < 2 {
                continue;
            }
            let words = conn.to_i32_vec()?;
            let mat_col = conn.conn_words - 2;
            let mut mats: Vec<i32> = words
                .chunks_exact(conn.conn_words)
                .map(|row| row[mat_col])
                .collect();
            mats.sort_unstable();
            mats.dedup();
            for mat in mats {
                if seen.insert(mat) {
                    out.push(mat);
                }
            }
        }
        Ok(out)
    }
}

fn descriptor_matches(name: &str, mesh_id: MeshId, sname_token: &str) -> bool {
    // Names from `ti_make_label_description` carry the descriptor
    // `[/Mesh-<id>/Sname-<class>/Scls-<superclass>/Mat-<matid>/]`. We
    // only need to confirm both the mesh-id and class tokens are
    // present; the order is fixed by the writer but matching by
    // substring is robust against future writer variations.
    let Some(open) = name.find('[') else {
        return false;
    };
    let desc = &name[open..];
    let mesh_token = format!("/Mesh-{}/", mesh_id.0);
    desc.contains(&mesh_token) && desc.contains(sname_token)
}

fn append_i32_array(
    bytes: &[u8],
    entry: &DirEntry,
    header: Header,
    out: &mut Vec<i32>,
) -> Result<()> {
    let value = ParamValue::decode(bytes, entry, header)?;
    let ParamValue::Array(arr) = value else {
        return Err(MiliError::MalformedDirectory(
            "TI accessor: expected array param",
        ));
    };
    if !matches!(arr.data_type, DataType::Int | DataType::Int4) {
        return Err(MiliError::MalformedDirectory(
            "TI accessor: array element type is not i32",
        ));
    }
    out.reserve(arr.atoms);
    let byteswap = !header.is_native_endian();
    crate::endian::for_each_swap::<i32, _>(arr.data, byteswap, |v| out.push(v));
    Ok(())
}

/// Filter inputs for a single-svar query against a [`Database`].
///
/// `svar` is the svar's short name; `class` the object-class short
/// name as written by `CLASS_DEF`. `labels` is a list of 1-based mili
/// object ids (the same id space as `Subrecord::id_blocks`); `None`
/// means "all objects". `states` is a non-empty list of state indices
/// into [`Database::states`]. `materials` selects element labels by
/// `mat_id` column from the class's `ELEM_CONNS`; combining with
/// `labels` takes the intersection. `ips` is 0-based integration-point
/// indices into the vec_array inner order; only valid against a
/// vec_array svar.
#[derive(Debug, Clone, Copy)]
pub struct QueryArgs<'a> {
    pub svar: &'a str,
    pub class: &'a str,
    pub labels: Option<&'a [i32]>,
    pub states: &'a [usize],
    pub materials: Option<&'a [i32]>,
    pub ips: Option<&'a [usize]>,
    /// Restrict to one named subrecord (the `subrec=` query kwarg).
    /// `None` means every matching subrecord.
    pub subrec: Option<&'a str>,
}

/// Per-state read context built before the parallel gather pass.
/// `plan` is rebased to this state's `state_data_start`; `mmap` and
/// `path` are owned (Arc-shared) handles for the slab reads.
struct StateCtx {
    plan: ReadPlan,
    mmap: Arc<Mmap>,
    path: PathBuf,
}

/// Byte offset of a state's data block inside its state file. Skips
/// the 8-byte per-state header (i32 srec_id + f32 time;
/// `reference/mili/src/mili.c:3042-3043`).
fn state_data_start(state: StateMeta) -> Result<u64> {
    let state_offset = u64::try_from(state.offset)
        .map_err(|_| MiliError::MalformedDirectory("state offset negative"))?;
    state_offset
        .checked_add(8)
        .ok_or(MiliError::MalformedDirectory(
            "state offset + header overflow",
        ))
}

fn label_range(id_blocks: &[(i32, i32)]) -> Vec<i32> {
    let mut out = Vec::new();
    for &(s, e) in id_blocks {
        for label in s..=e {
            out.push(label);
        }
    }
    out
}

fn slab_bytes<'a>(bytes: &'a [u8], path: &Path, start: usize, len: usize) -> Result<&'a [u8]> {
    let end = start.checked_add(len).ok_or(MiliError::MalformedDirectory(
        "state read: slab end overflow",
    ))?;
    bytes.get(start..end).ok_or_else(|| MiliError::Truncated {
        file: path.to_path_buf(),
        off: start as u64,
        need: len,
        got: bytes.len().saturating_sub(start),
    })
}

/// Build a state-file path from the `.A` path. `R.A` → `R.A00` etc.,
/// per `reference/mili/src/mili_util.c:881`.
fn state_file_path(a_path: &Path, suffix_width: u8, file_idx: i32) -> Option<PathBuf> {
    let name = a_path.file_name()?.to_str()?;
    let stem = name.strip_suffix('A')?;
    let width = usize::from(suffix_width);
    let suffix = format!("{file_idx:0width$}");
    Some(a_path.with_file_name(format!("{stem}{suffix}")))
}

fn mmap_read_only(path: &Path) -> Result<Mmap> {
    let file = File::open(path)?;
    // SAFETY: the mmap2 contract requires that the underlying file
    // not be mutated externally for the lifetime of the mapping. We
    // hold an open `File` handle through Database lifetime and never
    // expose write access, but the user could in principle modify
    // the file out-of-band. That mirrors the C reader's posture —
    // mili databases are not concurrent-write safe.
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(mmap)
}

#[cfg(test)]
mod tests {
    use super::descriptor_matches;
    use crate::mesh::MeshId;

    #[test]
    fn descriptor_matches_basic_pattern() {
        let name = "Element Labels[/Mesh-0/Sname-brick/Scls-M_HEX/Mat--1/]";
        assert!(descriptor_matches(name, MeshId(0), "Sname-brick"));
        assert!(!descriptor_matches(name, MeshId(1), "Sname-brick"));
        assert!(!descriptor_matches(name, MeshId(0), "Sname-node"));
    }

    #[test]
    fn descriptor_matches_rejects_bare_name() {
        // No descriptor bracket — should not match anything.
        assert!(!descriptor_matches(
            "Element Labels",
            MeshId(0),
            "Sname-brick"
        ));
    }

    #[test]
    fn descriptor_matches_substring_not_prefix() {
        // `Sname-bri` would substring-match `Sname-brick` if we weren't
        // careful; document the current behavior (substring) so callers
        // know to pass the full class name.
        let name = "Element Labels[/Mesh-2/Sname-brick/Scls-M_HEX/Mat-0/]";
        assert!(descriptor_matches(name, MeshId(2), "Sname-brick"));
    }
}
