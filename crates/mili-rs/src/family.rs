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

use memmap2::Mmap;

use crate::directory::{DirEntry, DirEntryType, Directory};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};
use crate::mesh::{self, Connectivity, MaterialId, MeshId, MeshTable, Nodes};
use crate::param::{DataType, ParamTable, ParamValue, ScalarValue};
use crate::state::{self, StateMapSource, StateMeta};

/// An opened mili family — the read-side handle through which all
/// parsed metadata and state byte ranges are reachable.
pub struct Database {
    a_path: PathBuf,
    a_mmap: Mmap,
    header: Header,
    directory: Directory,
    params: ParamTable,
    meshes: MeshTable,
    states: Vec<StateMeta>,
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
            states,
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
    let end = header.endianness;
    out.reserve(arr.atoms);
    for chunk in arr.data.chunks_exact(4) {
        let slot: [u8; 4] = chunk.try_into().expect("chunks_exact(4)");
        out.push(read_i32_4(end, slot));
    }
    Ok(())
}

fn read_i32_4(end: Endianness, slot: [u8; 4]) -> i32 {
    match end {
        Endianness::Big => i32::from_be_bytes(slot),
        Endianness::Little => i32::from_le_bytes(slot),
    }
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
