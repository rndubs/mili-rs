//! Mesh metadata: meshes, object classes, node coordinates, and element
//! connectivity.
//!
//! Step 5 of the implementation plan. The directory parser surfaces every
//! `CLASS_DEF`, `CLASS_IDENTS`, `NODES`, and `ELEM_CONNS` entry; this
//! module folds them into a typed [`MeshTable`] keyed by `(mesh_id,
//! classname)`, and exposes payload decoders for the two geometry entry
//! types.
//!
//! Coalescing semantics (see `planning/shared/entry-payloads.md`):
//!
//! - One `CLASS_DEF` per `(mesh_id, classname)`. `CLASS_DEF.MODIFIER1`
//!   is the superclass code and `STRING_QTY = 2` consumes a short name
//!   then a long name from the directory name pool.
//! - Zero or more `CLASS_IDENTS` per class. Each entry contributes one
//!   `(start_id, stop_id)` block; multiple entries represent a
//!   non-contiguous id range and are appended to the class's
//!   `id_blocks` in directory order.
//! - At most one `NODES` and at most one `ELEM_CONNS` per
//!   `(mesh_id, classname)` in the corpora we ship; we surface them by
//!   looking up the directory index, not by caching the bytes.
//!
//! All payload decoders return borrowed slices into the parent file's
//! mmap. Widening / byteswap is the caller's job until `MiliBuffer`
//! lands in Step 8.

use std::collections::HashMap;

use crate::directory::{DirEntry, DirEntryType, Directory, NamePool};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};

/// Mesh identifier, lifted from the geometry entries' `MODIFIER1` field.
///
/// Mesh ids are i32 on disk (4-byte M_INT) but stored in the directory's
/// widened i64 field. Negative values are rejected at table-build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(pub i32);

/// Material number, parsed from `MAT_NAME_<n>` / element-set names.
/// See `planning/shared/format.md` § "TI_PARAM-as-storage pattern".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId(pub i32);

/// Mili superclass codes from `planning/shared/format.md` § "Superclass
/// table". The numeric value is the on-disk code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Superclass {
    Unit = 0,
    Node = 1,
    Truss = 2,
    Beam = 3,
    Tri = 4,
    Quad = 5,
    Tet = 6,
    Pyramid = 7,
    Wedge = 8,
    Hex = 9,
    Mat = 10,
    Mesh = 11,
    Surface = 12,
    Particle = 13,
    Tet10 = 14,
    Inode = 15,
}

impl Superclass {
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Unit,
            1 => Self::Node,
            2 => Self::Truss,
            3 => Self::Beam,
            4 => Self::Tri,
            5 => Self::Quad,
            6 => Self::Tet,
            7 => Self::Pyramid,
            8 => Self::Wedge,
            9 => Self::Hex,
            10 => Self::Mat,
            11 => Self::Mesh,
            12 => Self::Surface,
            13 => Self::Particle,
            14 => Self::Tet10,
            15 => Self::Inode,
            _ => return None,
        })
    }

    /// Words per element in the on-disk connectivity stream. Returns 0
    /// for pseudo-classes that carry no geometry (`Unit`, `Node`,
    /// `Mat`, `Mesh`, `Surface`).
    pub fn conn_words(self) -> usize {
        match self {
            Self::Unit | Self::Node | Self::Mat | Self::Mesh | Self::Surface => 0,
            Self::Truss => 4,
            Self::Beam | Self::Tri => 5,
            Self::Quad | Self::Tet => 6,
            Self::Pyramid => 7,
            Self::Wedge => 8,
            Self::Hex => 10,
            Self::Particle | Self::Inode => 3,
            Self::Tet10 => 12,
        }
    }
}

/// One object class within one mesh.
#[derive(Debug, Clone)]
pub struct ObjectClass {
    pub mesh_id: MeshId,
    pub short_name: String,
    pub long_name: String,
    pub superclass: Superclass,
    /// Coalesced inclusive `[start_id, stop_id]` blocks from all
    /// `CLASS_IDENTS` entries for this class, in directory order.
    /// Empty if no idents were declared (in which case the class has
    /// zero elements).
    pub id_blocks: Vec<(i32, i32)>,
    /// Directory index of the `CLASS_DEF` entry — used to disambiguate
    /// duplicate names across meshes when looking up geometry entries.
    pub class_def_idx: usize,
}

impl ObjectClass {
    /// Total element count across all `id_blocks`.
    pub fn element_count(&self) -> u64 {
        self.id_blocks
            .iter()
            .map(|&(s, e)| (e as i64 - s as i64 + 1).max(0) as u64)
            .sum()
    }
}

/// Per-mesh roll-up of every object class declared for that mesh.
#[derive(Debug)]
pub struct Mesh {
    pub id: MeshId,
    /// Classes keyed by short name. Order of insertion is preserved
    /// via `class_order` for stable iteration.
    classes: HashMap<String, ObjectClass>,
    class_order: Vec<String>,
}

impl Mesh {
    pub fn class(&self, short_name: &str) -> Option<&ObjectClass> {
        self.classes.get(short_name)
    }

    pub fn class_names(&self) -> impl Iterator<Item = &str> {
        self.class_order.iter().map(String::as_str)
    }

    pub fn classes(&self) -> impl Iterator<Item = &ObjectClass> {
        self.class_order
            .iter()
            .map(|n| self.classes.get(n).expect("class index consistent"))
    }
}

/// The full set of meshes discovered in a directory.
///
/// Built once at open time from a parsed [`Directory`]. Cheap to query.
#[derive(Debug, Default)]
pub struct MeshTable {
    meshes: HashMap<MeshId, Mesh>,
    mesh_order: Vec<MeshId>,
    /// `(mesh_id, classname)` → directory index of the matching
    /// `NODES` entry, if any.
    nodes_index: HashMap<(MeshId, String), usize>,
    /// `(mesh_id, classname)` → directory index of the matching
    /// `ELEM_CONNS` entry, if any.
    conns_index: HashMap<(MeshId, String), usize>,
}

impl MeshTable {
    /// Walk a directory and fold every `CLASS_DEF` / `CLASS_IDENTS` /
    /// `NODES` / `ELEM_CONNS` entry into the table.
    ///
    /// Returns an error on malformed inputs (unknown superclass code,
    /// duplicate `CLASS_DEF` for the same `(mesh_id, classname)`,
    /// negative mesh id, or `CLASS_IDENTS` referring to a class that
    /// was never declared via `CLASS_DEF`).
    pub fn build(dir: &Directory) -> Result<Self> {
        let mut table = MeshTable::default();
        let names = &dir.names;

        for (idx, entry) in dir.entries.iter().enumerate() {
            if entry.entry_type == DirEntryType::ClassDef {
                table.add_class_def(idx, entry, names)?;
            }
        }
        // CLASS_IDENTS / NODES / ELEM_CONNS only make sense once the
        // classes they reference exist. Run them in a second pass so
        // declaration order in the directory doesn't matter.
        for (idx, entry) in dir.entries.iter().enumerate() {
            match entry.entry_type {
                DirEntryType::ClassIdents => table.add_class_idents(idx, entry, names)?,
                DirEntryType::Nodes => table.add_nodes(idx, entry, names)?,
                DirEntryType::ElemConns => table.add_elem_conns(idx, entry, names)?,
                _ => {}
            }
        }
        Ok(table)
    }

    fn add_class_def(&mut self, idx: usize, entry: &DirEntry, names: &NamePool) -> Result<()> {
        // CLASS_DEF: MODIFIER1 = superclass, STRING_QTY = 2
        // (short_name, long_name). The reference notes call out that
        // mesh id does **not** live in CLASS_DEF — the class is
        // claimed by whichever mesh first emits a CLASS_IDENTS / NODES
        // / ELEM_CONNS entry that names it. We tag it with MeshId(-1)
        // as a sentinel and rewrite the field the first time a
        // geometry entry pins it to a real mesh.
        if entry.name_count < 2 {
            return Err(MiliError::MalformedDirectory(
                "CLASS_DEF entry has fewer than two names",
            ));
        }
        let superclass = Superclass::from_code(entry.modifier1).ok_or(
            MiliError::MalformedDirectory("CLASS_DEF: unknown superclass code"),
        )?;
        let short = names.get(entry.name_start as usize).to_owned();
        let long = names.get(entry.name_start as usize + 1).to_owned();

        let class = ObjectClass {
            mesh_id: MeshId(-1),
            short_name: short.clone(),
            long_name: long,
            superclass,
            id_blocks: Vec::new(),
            class_def_idx: idx,
        };
        // Stash under the sentinel mesh id; the resolve pass below
        // moves it to the real mesh once a geometry entry binds it.
        let sentinel = MeshId(-1);
        let mesh = self.mesh_entry(sentinel);
        if mesh.classes.contains_key(&short) {
            return Err(MiliError::MalformedDirectory(
                "duplicate CLASS_DEF for the same class name",
            ));
        }
        mesh.class_order.push(short.clone());
        mesh.classes.insert(short, class);
        Ok(())
    }

    fn mesh_entry(&mut self, id: MeshId) -> &mut Mesh {
        if !self.meshes.contains_key(&id) {
            self.mesh_order.push(id);
            self.meshes.insert(
                id,
                Mesh {
                    id,
                    classes: HashMap::new(),
                    class_order: Vec::new(),
                },
            );
        }
        self.meshes.get_mut(&id).expect("just inserted")
    }

    fn resolve_class(&mut self, mesh_id: MeshId, class_name: &str) -> Result<()> {
        // If the class already lives under `mesh_id`, nothing to do.
        if self
            .meshes
            .get(&mesh_id)
            .is_some_and(|m| m.classes.contains_key(class_name))
        {
            return Ok(());
        }
        // Look for it in the sentinel bucket.
        let sentinel = MeshId(-1);
        let Some(mut class) = self.meshes.get_mut(&sentinel).and_then(|m| {
            if m.classes.contains_key(class_name) {
                // Removal: also drop from class_order.
                m.class_order.retain(|n| n != class_name);
                m.classes.remove(class_name)
            } else {
                None
            }
        }) else {
            return Err(MiliError::MalformedDirectory(
                "geometry entry references a class with no CLASS_DEF",
            ));
        };
        class.mesh_id = mesh_id;
        let mesh = self.mesh_entry(mesh_id);
        mesh.class_order.push(class_name.to_owned());
        mesh.classes.insert(class_name.to_owned(), class);
        Ok(())
    }

    fn add_class_idents(&mut self, _idx: usize, entry: &DirEntry, names: &NamePool) -> Result<()> {
        let (mesh_id, classname) = mesh_and_classname(entry, names)?;
        self.resolve_class(mesh_id, &classname)?;
        // CLASS_IDENTS modifier1 == mesh_id, modifier2 == element count.
        // The payload (3 × M_INT: superclass, start_id, stop_id) is not
        // needed here — the directory entry's modifier fields suffice
        // for table building. We do still want the `[start, stop]`
        // range, but it lives in the payload. Defer the actual decode
        // until `decode_class_idents`, called from the second pass
        // below.
        Ok(())
    }

    fn add_nodes(&mut self, idx: usize, entry: &DirEntry, names: &NamePool) -> Result<()> {
        let (mesh_id, classname) = mesh_and_classname(entry, names)?;
        self.resolve_class(mesh_id, &classname)?;
        let key = (mesh_id, classname);
        if self.nodes_index.insert(key, idx).is_some() {
            return Err(MiliError::MalformedDirectory(
                "duplicate NODES entry for the same (mesh, class)",
            ));
        }
        Ok(())
    }

    fn add_elem_conns(&mut self, idx: usize, entry: &DirEntry, names: &NamePool) -> Result<()> {
        let (mesh_id, classname) = mesh_and_classname(entry, names)?;
        self.resolve_class(mesh_id, &classname)?;
        let key = (mesh_id, classname);
        if self.conns_index.insert(key, idx).is_some() {
            return Err(MiliError::MalformedDirectory(
                "duplicate ELEM_CONNS entry for the same (mesh, class)",
            ));
        }
        Ok(())
    }

    /// Append id-block ranges to each class by decoding every
    /// `CLASS_IDENTS` payload. Call after [`Self::build`] when you
    /// have the file bytes; safe to call multiple times only at the
    /// cost of duplicate ranges, so the open path calls it exactly
    /// once.
    pub fn load_ident_ranges(
        &mut self,
        bytes: &[u8],
        header: Header,
        dir: &Directory,
    ) -> Result<()> {
        for entry in &dir.entries {
            if entry.entry_type != DirEntryType::ClassIdents {
                continue;
            }
            let (mesh_id, classname) = mesh_and_classname(entry, &dir.names)?;
            let block = decode_class_idents(bytes, entry, header)?;
            let mesh = self
                .meshes
                .get_mut(&mesh_id)
                .ok_or(MiliError::MalformedDirectory(
                    "load_ident_ranges: mesh missing",
                ))?;
            let class = mesh
                .classes
                .get_mut(&classname)
                .ok_or(MiliError::MalformedDirectory(
                    "load_ident_ranges: class missing",
                ))?;
            class.id_blocks.push(block);
        }
        Ok(())
    }

    pub fn meshes(&self) -> impl Iterator<Item = &Mesh> {
        self.mesh_order
            .iter()
            .filter(|m| **m != MeshId(-1))
            .map(|id| self.meshes.get(id).expect("mesh order consistent"))
    }

    pub fn mesh(&self, id: MeshId) -> Option<&Mesh> {
        self.meshes.get(&id)
    }

    /// Directory index of the `NODES` entry for `(mesh_id, classname)`,
    /// or `None` if no such entry was declared.
    pub fn nodes_entry_index(&self, mesh_id: MeshId, classname: &str) -> Option<usize> {
        self.nodes_index
            .get(&(mesh_id, classname.to_owned()))
            .copied()
    }

    /// Directory index of the `ELEM_CONNS` entry for
    /// `(mesh_id, classname)`, or `None` if no such entry was declared.
    pub fn conns_entry_index(&self, mesh_id: MeshId, classname: &str) -> Option<usize> {
        self.conns_index
            .get(&(mesh_id, classname.to_owned()))
            .copied()
    }

    /// Every class across every mesh, in directory-discovery order.
    pub fn classes(&self) -> impl Iterator<Item = &ObjectClass> {
        self.meshes().flat_map(Mesh::classes)
    }
}

fn mesh_and_classname(entry: &DirEntry, names: &NamePool) -> Result<(MeshId, String)> {
    if entry.name_count < 1 {
        return Err(MiliError::MalformedDirectory(
            "geometry entry missing class name",
        ));
    }
    let raw = entry.modifier1;
    let id = i32::try_from(raw)
        .map_err(|_| MiliError::MalformedDirectory("mesh id out of i32 range"))?;
    if id < 0 {
        return Err(MiliError::MalformedDirectory("negative mesh id"));
    }
    Ok((MeshId(id), names.get(entry.name_start as usize).to_owned()))
}

/// Decoded `NODES` payload — a header (`[start_node, stop_node]`) plus
/// a borrowed byte slice over the coordinate data.
///
/// The coordinate stride is `dims * sizeof(M_FLOAT)` per node; the
/// caller obtains `dims` from `Database::mesh_dimensions()`.
#[derive(Debug)]
pub struct Nodes<'a> {
    /// Inclusive global id of the first node in this entry.
    pub start_id: i32,
    /// Inclusive global id of the last node in this entry.
    pub stop_id: i32,
    /// Spatial dimensions per node — copied from the database scalar
    /// param `"mesh dimensions"` for caller convenience.
    pub dimensions: i32,
    /// Native-endian or byte-swapped raw bytes for the coordinate
    /// block. Length is `(stop_id - start_id + 1) * dimensions *
    /// header.float_size()`.
    pub data: &'a [u8],
    /// `Endianness` of the bytes — needed when widening to `f32`.
    pub endianness: Endianness,
}

impl Nodes<'_> {
    pub fn node_count(&self) -> i64 {
        (self.stop_id as i64) - (self.start_id as i64) + 1
    }

    /// Decode the raw byte slice into an owned `Vec<f32>`.
    ///
    /// Layout is `[node][dim]`. Returns an error on length mismatch.
    pub fn to_f32_vec(&self) -> Result<Vec<f32>> {
        if !self.data.len().is_multiple_of(4) {
            return Err(MiliError::MalformedDirectory(
                "nodes: data length not a multiple of 4",
            ));
        }
        let n = self.data.len() / 4;
        let mut out = Vec::with_capacity(n);
        for chunk in self.data.chunks_exact(4) {
            let arr: [u8; 4] = chunk.try_into().expect("chunks_exact(4)");
            out.push(match self.endianness {
                Endianness::Big => f32::from_be_bytes(arr),
                Endianness::Little => f32::from_le_bytes(arr),
            });
        }
        Ok(out)
    }
}

/// Decode a `NODES` directory entry against the file bytes.
///
/// `dimensions` must come from the `"mesh dimensions"` scalar param —
/// it's not redundantly stored in the entry. See
/// `planning/shared/entry-payloads.md` § `NODES`.
pub fn decode_nodes<'a>(
    bytes: &'a [u8],
    entry: &DirEntry,
    header: Header,
    dimensions: i32,
) -> Result<Nodes<'a>> {
    if entry.entry_type != DirEntryType::Nodes {
        return Err(MiliError::MalformedDirectory(
            "decode_nodes: wrong entry type",
        ));
    }
    if dimensions <= 0 {
        return Err(MiliError::MalformedDirectory("mesh dimensions <= 0"));
    }
    let raw = payload(bytes, entry)?;
    let int_w = header.int_size();
    if raw.len() < 2 * int_w {
        return Err(MiliError::MalformedDirectory(
            "NODES payload shorter than [start, stop] prefix",
        ));
    }
    let start_id = read_i32(header.endianness, &raw[..int_w], int_w)?;
    let stop_id = read_i32(header.endianness, &raw[int_w..2 * int_w], int_w)?;
    if stop_id < start_id {
        return Err(MiliError::MalformedDirectory("NODES: stop < start"));
    }
    let count: i64 = (stop_id as i64) - (start_id as i64) + 1;
    let body_floats = count
        .checked_mul(i64::from(dimensions))
        .ok_or(MiliError::MalformedDirectory("NODES: count*dim overflow"))?;
    let body_bytes = body_floats
        .checked_mul(header.float_size() as i64)
        .and_then(|b| usize::try_from(b).ok())
        .ok_or(MiliError::MalformedDirectory("NODES: byte length overflow"))?;
    let end = 2 * int_w + body_bytes;
    if raw.len() < end {
        return Err(MiliError::MalformedDirectory(
            "NODES: payload shorter than declared body",
        ));
    }
    Ok(Nodes {
        start_id,
        stop_id,
        dimensions,
        data: &raw[2 * int_w..end],
        endianness: header.endianness,
    })
}

fn decode_class_idents(bytes: &[u8], entry: &DirEntry, header: Header) -> Result<(i32, i32)> {
    if entry.entry_type != DirEntryType::ClassIdents {
        return Err(MiliError::MalformedDirectory(
            "decode_class_idents: wrong entry type",
        ));
    }
    let raw = payload(bytes, entry)?;
    let int_w = header.int_size();
    if raw.len() < 3 * int_w {
        return Err(MiliError::MalformedDirectory(
            "CLASS_IDENTS payload shorter than three M_INTs",
        ));
    }
    // The first int is the superclass code; the C reader cross-checks
    // it against the CLASS_DEF's superclass (`mesh_u.c:4062-4095`). We
    // accept any value here — the class is already typed by its
    // CLASS_DEF entry — and just decode the id range.
    let _superclass = read_i32(header.endianness, &raw[..int_w], int_w)?;
    let start_id = read_i32(header.endianness, &raw[int_w..2 * int_w], int_w)?;
    let stop_id = read_i32(header.endianness, &raw[2 * int_w..3 * int_w], int_w)?;
    if stop_id < start_id {
        return Err(MiliError::MalformedDirectory(
            "CLASS_IDENTS: stop_id < start_id",
        ));
    }
    Ok((start_id, stop_id))
}

/// Decoded `ELEM_CONNS` payload — superclass code, the list of
/// `(start, stop)` id blocks, and the connectivity stream.
#[derive(Debug)]
pub struct Connectivity<'a> {
    pub superclass: Superclass,
    /// `(start_id, stop_id)` blocks covered by this entry.
    pub blocks: Vec<(i32, i32)>,
    /// Words-per-element on disk for this superclass.
    pub conn_words: usize,
    /// Raw bytes for the connectivity stream. Length is
    /// `sum(block_len) * conn_words * header.int_size()`.
    pub data: &'a [u8],
    pub endianness: Endianness,
}

impl Connectivity<'_> {
    /// Decode the raw byte stream into an owned `Vec<i32>` in
    /// row-major `[element][word]` order.
    pub fn to_i32_vec(&self) -> Result<Vec<i32>> {
        if !self.data.len().is_multiple_of(4) {
            return Err(MiliError::MalformedDirectory(
                "conn: data length not a multiple of 4",
            ));
        }
        let n = self.data.len() / 4;
        let mut out = Vec::with_capacity(n);
        for chunk in self.data.chunks_exact(4) {
            let arr: [u8; 4] = chunk.try_into().expect("chunks_exact(4)");
            out.push(match self.endianness {
                Endianness::Big => i32::from_be_bytes(arr),
                Endianness::Little => i32::from_le_bytes(arr),
            });
        }
        Ok(out)
    }
}

/// Decode an `ELEM_CONNS` directory entry against the file bytes.
pub fn decode_elem_conns<'a>(
    bytes: &'a [u8],
    entry: &DirEntry,
    header: Header,
) -> Result<Connectivity<'a>> {
    if entry.entry_type != DirEntryType::ElemConns {
        return Err(MiliError::MalformedDirectory(
            "decode_elem_conns: wrong entry type",
        ));
    }
    let raw = payload(bytes, entry)?;
    let int_w = header.int_size();
    let end = header.endianness;
    if raw.len() < 2 * int_w {
        return Err(MiliError::MalformedDirectory(
            "ELEM_CONNS payload shorter than [superclass, qty_blocks] header",
        ));
    }
    let super_code = read_i32(end, &raw[..int_w], int_w)?;
    let superclass = Superclass::from_code(i64::from(super_code)).ok_or(
        MiliError::MalformedDirectory("ELEM_CONNS: unknown superclass code"),
    )?;
    let qty_blocks = read_i32(end, &raw[int_w..2 * int_w], int_w)?;
    if qty_blocks < 0 {
        return Err(MiliError::MalformedDirectory("ELEM_CONNS: qty_blocks < 0"));
    }
    let qty_blocks_us = qty_blocks as usize;
    let blocks_bytes =
        qty_blocks_us
            .checked_mul(2 * int_w)
            .ok_or(MiliError::MalformedDirectory(
                "ELEM_CONNS: block list size overflow",
            ))?;
    let header_bytes = 2 * int_w + blocks_bytes;
    if raw.len() < header_bytes {
        return Err(MiliError::MalformedDirectory(
            "ELEM_CONNS: block list past payload end",
        ));
    }
    let mut blocks = Vec::with_capacity(qty_blocks_us);
    let mut total_elems: u64 = 0;
    for i in 0..qty_blocks_us {
        let base = 2 * int_w + i * 2 * int_w;
        let start = read_i32(end, &raw[base..base + int_w], int_w)?;
        let stop = read_i32(end, &raw[base + int_w..base + 2 * int_w], int_w)?;
        if stop < start {
            return Err(MiliError::MalformedDirectory(
                "ELEM_CONNS: block stop < start",
            ));
        }
        total_elems += (stop as i64 - start as i64 + 1) as u64;
        blocks.push((start, stop));
    }
    let conn_words = superclass.conn_words();
    let body_bytes = (total_elems as u128)
        .checked_mul(conn_words as u128)
        .and_then(|b| b.checked_mul(int_w as u128))
        .and_then(|b| usize::try_from(b).ok())
        .ok_or(MiliError::MalformedDirectory(
            "ELEM_CONNS: body byte length overflow",
        ))?;
    let body_end = header_bytes + body_bytes;
    if raw.len() < body_end {
        return Err(MiliError::MalformedDirectory(
            "ELEM_CONNS: connectivity stream past payload end",
        ));
    }
    Ok(Connectivity {
        superclass,
        blocks,
        conn_words,
        data: &raw[header_bytes..body_end],
        endianness: end,
    })
}

fn payload<'a>(bytes: &'a [u8], entry: &DirEntry) -> Result<&'a [u8]> {
    let off = usize::try_from(entry.offset)
        .map_err(|_| MiliError::MalformedDirectory("mesh: negative offset"))?;
    let len = usize::try_from(entry.length)
        .map_err(|_| MiliError::MalformedDirectory("mesh: negative length"))?;
    let end = off.checked_add(len).ok_or(MiliError::MalformedDirectory(
        "mesh: offset+length overflow",
    ))?;
    bytes
        .get(off..end)
        .ok_or(MiliError::MalformedDirectory("mesh: payload past EOF"))
}

fn read_i32(end: Endianness, slot: &[u8], width: usize) -> Result<i32> {
    match width {
        4 => {
            let arr: &[u8; 4] = slot
                .try_into()
                .map_err(|_| MiliError::MalformedDirectory("read_i32: bad slice"))?;
            Ok(end.read_i32(arr))
        }
        8 => {
            let arr: &[u8; 8] = slot
                .try_into()
                .map_err(|_| MiliError::MalformedDirectory("read_i32: bad slice"))?;
            let v = end.read_i64(arr);
            i32::try_from(v)
                .map_err(|_| MiliError::MalformedDirectory("read_i32: value exceeds i32"))
        }
        _ => Err(MiliError::MalformedDirectory("read_i32: unsupported width")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{ByteRange, DirEntry, DirEntryType, NamePool};
    use crate::header::{Endianness, PartitionScheme, PrecisionLimit};

    fn h() -> Header {
        Header {
            header_version: 3,
            dir_version: 3,
            endianness: Endianness::Little,
            precision_limit: PrecisionLimit::Double,
            suffix_width: 2,
            partition_scheme: PartitionScheme::StateCount,
        }
    }

    fn entry(
        t: DirEntryType,
        m1: i64,
        m2: i64,
        off: i64,
        len: i64,
        name_start: u32,
        name_count: u32,
    ) -> DirEntry {
        DirEntry {
            entry_type: t,
            modifier1: m1,
            modifier2: m2,
            string_qty: i64::from(name_count),
            offset: off,
            length: len,
            name_start,
            name_count,
        }
    }

    fn make_pool(names: &[&str]) -> NamePool {
        let mut buf = Vec::new();
        for n in names {
            buf.extend_from_slice(n.as_bytes());
            buf.push(0);
        }
        NamePool::parse(&buf, names.len() as u32).unwrap()
    }

    #[test]
    fn superclass_codes_round_trip() {
        for code in [0i64, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
            assert!(Superclass::from_code(code).is_some(), "code {code}");
        }
        assert!(Superclass::from_code(99).is_none());
        assert_eq!(Superclass::Hex.conn_words(), 10);
        assert_eq!(Superclass::Node.conn_words(), 0);
    }

    #[test]
    fn build_table_collects_class_def_and_idents() {
        // Directory layout (file bytes):
        //   [0..16) header padding
        //   [16..28) CLASS_IDENTS payload: superclass=9, start=1, stop=8 (12 bytes)
        let names = ["hex", "Hexahedral", "hex"];
        let pool = make_pool(&names);
        let mut bytes = vec![0u8; 64];
        // Idents payload at offset 16: superclass=9, start=1, stop=8
        bytes[16..20].copy_from_slice(&9i32.to_le_bytes());
        bytes[20..24].copy_from_slice(&1i32.to_le_bytes());
        bytes[24..28].copy_from_slice(&8i32.to_le_bytes());

        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![
                entry(DirEntryType::ClassDef, 9, 9, 0, 0, 0, 2),
                entry(DirEntryType::ClassIdents, 0, 8, 16, 12, 2, 1),
            ],
            names: pool,
        };
        let mut table = MeshTable::build(&dir).unwrap();
        table.load_ident_ranges(&bytes, h(), &dir).unwrap();

        let mesh = table.mesh(MeshId(0)).expect("mesh 0 present");
        let class = mesh.class("hex").expect("hex class present");
        assert_eq!(class.superclass, Superclass::Hex);
        assert_eq!(class.long_name, "Hexahedral");
        assert_eq!(class.id_blocks, vec![(1, 8)]);
        assert_eq!(class.element_count(), 8);
    }

    #[test]
    fn build_table_coalesces_multiple_idents() {
        let names = ["brick", "Brick", "brick", "brick"];
        let pool = make_pool(&names);
        let mut bytes = vec![0u8; 128];
        // Two CLASS_IDENTS payloads — non-contiguous id ranges.
        for (off, sc, s, e) in [(16i32, 9i32, 1i32, 4i32), (32, 9, 100, 103)] {
            let off = off as usize;
            bytes[off..off + 4].copy_from_slice(&sc.to_le_bytes());
            bytes[off + 4..off + 8].copy_from_slice(&s.to_le_bytes());
            bytes[off + 8..off + 12].copy_from_slice(&e.to_le_bytes());
        }

        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![
                entry(DirEntryType::ClassDef, 9, 9, 0, 0, 0, 2),
                entry(DirEntryType::ClassIdents, 0, 4, 16, 12, 2, 1),
                entry(DirEntryType::ClassIdents, 0, 4, 32, 12, 3, 1),
            ],
            names: pool,
        };
        let mut table = MeshTable::build(&dir).unwrap();
        table.load_ident_ranges(&bytes, h(), &dir).unwrap();
        let class = table.mesh(MeshId(0)).unwrap().class("brick").unwrap();
        assert_eq!(class.id_blocks, vec![(1, 4), (100, 103)]);
        assert_eq!(class.element_count(), 8);
    }

    #[test]
    fn build_table_rejects_duplicate_class_def() {
        let names = ["x", "X", "x", "X"];
        let pool = make_pool(&names);
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![
                entry(DirEntryType::ClassDef, 1, 1, 0, 0, 0, 2),
                entry(DirEntryType::ClassDef, 1, 1, 0, 0, 2, 2),
            ],
            names: pool,
        };
        assert!(MeshTable::build(&dir).is_err());
    }

    #[test]
    fn decode_nodes_round_trip() {
        // start=1, stop=2, dims=3 → 2*3 = 6 floats.
        let coords: [f32; 6] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let mut bytes = vec![0u8; 64];
        bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
        bytes[20..24].copy_from_slice(&2i32.to_le_bytes());
        for (i, c) in coords.iter().enumerate() {
            let off = 24 + i * 4;
            bytes[off..off + 4].copy_from_slice(&c.to_le_bytes());
        }
        let len = 8 + 24; // 2 ints + 6 floats
        let e = entry(DirEntryType::Nodes, 0, 2, 16, len, 0, 1);
        let nodes = decode_nodes(&bytes, &e, h(), 3).unwrap();
        assert_eq!(nodes.start_id, 1);
        assert_eq!(nodes.stop_id, 2);
        assert_eq!(nodes.node_count(), 2);
        assert_eq!(nodes.data.len(), 24);
        assert_eq!(nodes.to_f32_vec().unwrap(), coords.to_vec());
    }

    #[test]
    fn decode_nodes_rejects_short_payload() {
        let mut bytes = vec![0u8; 32];
        bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
        bytes[20..24].copy_from_slice(&5i32.to_le_bytes());
        // length doesn't cover the declared 5 nodes' worth of f32s.
        let e = entry(DirEntryType::Nodes, 0, 5, 16, 8, 0, 1);
        assert!(decode_nodes(&bytes, &e, h(), 3).is_err());
    }

    #[test]
    fn decode_elem_conns_round_trip() {
        // One M_TRUSS block, ids 1..=2 → 2 elements × 4 words = 8 ints.
        let conn: [i32; 8] = [10, 11, 1, 0, 12, 13, 1, 0];
        let mut bytes = vec![0u8; 96];
        // payload at offset 16
        bytes[16..20].copy_from_slice(&(Superclass::Truss as i32).to_le_bytes());
        bytes[20..24].copy_from_slice(&1i32.to_le_bytes()); // qty_blocks
        bytes[24..28].copy_from_slice(&1i32.to_le_bytes()); // start
        bytes[28..32].copy_from_slice(&2i32.to_le_bytes()); // stop
        for (i, w) in conn.iter().enumerate() {
            let off = 32 + i * 4;
            bytes[off..off + 4].copy_from_slice(&w.to_le_bytes());
        }
        let len = 4 * 4 + 8 * 4; // header(2) + blocks(2) + conn(8)
        let e = entry(DirEntryType::ElemConns, 0, 2, 16, len, 0, 1);
        let c = decode_elem_conns(&bytes, &e, h()).unwrap();
        assert_eq!(c.superclass, Superclass::Truss);
        assert_eq!(c.blocks, vec![(1, 2)]);
        assert_eq!(c.conn_words, 4);
        assert_eq!(c.to_i32_vec().unwrap(), conn.to_vec());
    }

    #[test]
    fn decode_elem_conns_rejects_truncated_block_list() {
        let mut bytes = vec![0u8; 32];
        bytes[16..20].copy_from_slice(&(Superclass::Hex as i32).to_le_bytes());
        bytes[20..24].copy_from_slice(&3i32.to_le_bytes()); // claim 3 blocks
        let e = entry(DirEntryType::ElemConns, 0, 0, 16, 16, 0, 1);
        assert!(decode_elem_conns(&bytes, &e, h()).is_err());
    }
}
