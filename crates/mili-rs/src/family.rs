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
use crate::header::Header;
use crate::mesh::{self, Connectivity, MaterialId, MeshId, MeshTable, Nodes};
use crate::param::{DataType, ParamTable, ParamValue, ScalarValue};
use crate::query::{plan_state_svar, Filter, ReadPlan, StateValues};
use crate::srec::SrecTable;
use crate::state::{self, StateMapSource, StateMeta};
use crate::svar::{NumType, SvarTable};

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
        let srecs = SrecTable::build(&a_mmap, &directory, header)?;

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
        Ok(if any { Some(out) } else { None })
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
            let Some(setname) = name.strip_prefix("IntLabel_es_") else {
                continue;
            };
            let mut values = Vec::new();
            append_i32_array(&self.a_mmap, entry, self.header, &mut values)?;
            out.insert(setname.to_owned(), values);
        }
        Ok(out)
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

        let filter = Filter {
            labels: resolved_labels.as_deref(),
            ips: args.ips,
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
        let plan = plan_state_svar(
            srec,
            &self.svars,
            args.svar,
            args.class,
            first_data_start,
            filter,
        )?;

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

        Ok(match plan.num_type {
            NumType::Float4 => gather!(f32, F32),
            NumType::Float8 => gather!(f64, F64),
            NumType::Int4 => gather!(i32, I32),
            NumType::Int8 => gather!(i64, I64),
        })
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
    /// An element-set name that parses as an `i32` is treated as a
    /// material number; the IP ids are the payload minus the trailing
    /// count entry. Sets whose names do not parse as integers are
    /// skipped.
    ///
    /// `reference/mili-python/src/mili/miliinternal.py:463-474`.
    pub fn integration_points(&self) -> Result<HashMap<MaterialId, Vec<i32>>> {
        let sets = self.element_sets()?;
        let mut out: HashMap<MaterialId, Vec<i32>> = HashMap::new();
        for (setname, values) in sets {
            let Ok(mat) = setname.parse::<i32>() else {
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
