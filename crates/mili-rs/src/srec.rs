//! `STATE_REC_DATA` payload decoding and derived per-subrecord lump
//! metrics.
//!
//! The state-record schema is a dual stream (an int stream and a char
//! stream), per `planning/shared/entry-payloads.md` § `STATE_REC_DATA`.
//! One [`Srec`] per entry, each holding a [`Subrecord`] list. Names of
//! object classes and svars are stored on disk; the caller resolves
//! them against the mesh and svar tables at query time.
//!
//! The per-subrecord `lump_offsets`, `lump_sizes`, and `lump_atoms`
//! arrays from the C library (`reference/mili/src/srec.c:1409+`,
//! cited via `.gitmodules`) are not written to disk — they are
//! derived at load time from each svar's resolved atom count and
//! numeric width by [`derive_lumps`].
//!
//! The byte-layout matrix in `planning/shared/format.md` §
//! "Subrecord byte-layout matrix" pins down how the query layer
//! composes the lumps into concrete byte addresses:
//!
//! - `RESULT_ORDERED`: svar `s`'s slab starts at `N * lump_offsets[s]`
//!   from the subrec start; object `j` within it is at
//!   `j * lump_sizes[s]`.
//! - `OBJECT_ORDERED`: object `j` starts at `j * stride`, where
//!   `stride = lump_offsets[K-1] + lump_sizes[K-1]`; svar `s` within
//!   that object is at offset `lump_offsets[s]`.

use std::collections::HashMap;

use crate::directory::{DirEntry, DirEntryType, Directory};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};

/// Subrecord-layout flag from the C library's `subrec_layout` enum
/// (`mili_enum.h:41-45`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Organization {
    ResultOrdered = 0,
    ObjectOrdered = 1,
}

impl Organization {
    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::ResultOrdered,
            1 => Self::ObjectOrdered,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Subrecord {
    pub name: String,
    /// Raw mesh-class short name as written on disk; resolve against
    /// the mesh table at query time.
    pub mclass: String,
    pub organization: Organization,
    /// Raw svar names. Resolve against the [`crate::SvarTable`] at
    /// query time.
    pub svar_names: Vec<String>,
    /// Inclusive `(start, stop)` object-id ranges as written on disk.
    /// These are 1-based mili ids per
    /// `reference/mili-python/src/mili/afileIO.py:444-445`; the query
    /// layer normalises to 0-based ordinals.
    pub id_blocks: Vec<(i32, i32)>,
}

impl Subrecord {
    /// Total objects covered by this subrecord, summing the inclusive
    /// `id_blocks`. Returns 0 if blocks is empty.
    pub fn object_count(&self) -> u64 {
        self.id_blocks
            .iter()
            .map(|&(s, e)| (e as i64 - s as i64 + 1).max(0) as u64)
            .sum()
    }
}

#[derive(Debug, Clone)]
pub struct Srec {
    pub srec_id: i32,
    pub mesh_id: i32,
    /// Bytes per state for this srec, as written on disk (the third
    /// int in the int-stream header).
    pub srec_size: i32,
    pub subrecords: Vec<Subrecord>,
}

/// Per-subrecord layout metrics, populated by [`derive_lumps`].
///
/// Each field is a parallel vector of length `K` (the subrecord's svar
/// count). See the module-level docs for how the query layer composes
/// these with the [`Subrecord::object_count`] under each
/// [`Organization`].
#[derive(Debug, Clone)]
pub struct Lumps {
    /// Atoms per object per svar.
    pub atoms: Vec<usize>,
    /// Bytes per object per svar — `atoms[s] * svar.num_type.width()`.
    pub sizes: Vec<usize>,
    /// Prefix sum of `sizes`; `offsets[s] = sum_{i<s} sizes[i]`. Use
    /// the formulas in the module-level docs to fold this into a byte
    /// address.
    pub offsets: Vec<usize>,
}

impl Lumps {
    /// Total bytes per object across all svars in the subrecord.
    /// Useful as the stride in `OBJECT_ORDERED` layouts.
    pub fn bytes_per_object(&self) -> usize {
        self.sizes.iter().sum()
    }
}

/// Compute per-svar [`Lumps`] from each svar's atom count and numeric
/// width. The values are independent of [`Organization`] — only the
/// query layer's offset formula changes. See module-level docs.
pub fn derive_lumps(svar_atoms: &[usize], svar_widths: &[usize]) -> Lumps {
    assert_eq!(
        svar_atoms.len(),
        svar_widths.len(),
        "derive_lumps: atom and width vectors must align"
    );
    let k = svar_atoms.len();
    let mut sizes = Vec::with_capacity(k);
    let mut offsets = Vec::with_capacity(k);
    let mut acc: usize = 0;
    for i in 0..k {
        let s = svar_atoms[i].saturating_mul(svar_widths[i]);
        sizes.push(s);
        offsets.push(acc);
        acc = acc.saturating_add(s);
    }
    Lumps {
        atoms: svar_atoms.to_vec(),
        sizes,
        offsets,
    }
}

/// All srecs parsed from the directory's `STATE_REC_DATA` entries,
/// indexed by `srec_id`.
#[derive(Debug, Default)]
pub struct SrecTable {
    by_id: HashMap<i32, Srec>,
    order: Vec<i32>,
}

impl SrecTable {
    pub fn build(bytes: &[u8], dir: &Directory, header: Header) -> Result<Self> {
        let mut table = SrecTable::default();
        for entry in &dir.entries {
            if entry.entry_type == DirEntryType::StateRecData {
                let srec = parse_srec_entry(bytes, entry, header)?;
                let id = srec.srec_id;
                if !table.by_id.contains_key(&id) {
                    table.order.push(id);
                }
                table.by_id.insert(id, srec);
            }
        }
        Ok(table)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn get(&self, srec_id: i32) -> Option<&Srec> {
        self.by_id.get(&srec_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Srec> {
        self.order
            .iter()
            .map(|id| self.by_id.get(id).expect("srec order index consistent"))
    }

    /// Synthesise `id_blocks = [(1, 1)]` for every subrec whose mclass
    /// has `Superclass::Mesh` and whose on-disk `id_blocks` list is
    /// empty. Mirrors mili-python's parser:
    /// `reference/mili-python/src/mili/afileIO.py:439-441` — for
    /// `M_MESH` superclass the writer emits `block_count = 0` and no
    /// id-block pairs, but the layout always carries one object's worth
    /// of data per state. Without this fix, every subrec that appears
    /// after a glob / mesh subrec in directory order ends up offset by
    /// the swallowed M_MESH bytes (e.g. basic1 mis-reads `nodvel` by 80
    /// bytes — `1 * 19 * 4` for glob + `1 * 1 * 4` for cpu_time).
    pub fn patch_m_mesh_classes(&mut self, meshes: &crate::mesh::MeshTable) {
        for srec_id in &self.order {
            let Some(srec) = self.by_id.get_mut(srec_id) else {
                continue;
            };
            let mesh_id = crate::mesh::MeshId(srec.mesh_id);
            let Some(mesh) = meshes.mesh(mesh_id) else {
                continue;
            };
            for sub in &mut srec.subrecords {
                if !sub.id_blocks.is_empty() {
                    continue;
                }
                if mesh
                    .class(&sub.mclass)
                    .is_some_and(|c| c.superclass == crate::mesh::Superclass::Mesh)
                {
                    sub.id_blocks.push((1, 1));
                }
            }
        }
    }
}

fn parse_srec_entry(bytes: &[u8], entry: &DirEntry, header: Header) -> Result<Srec> {
    let raw = payload(bytes, entry)?;
    let int_w = header.int_size();
    let end = header.endianness;
    if int_w != 4 {
        return Err(MiliError::MalformedDirectory(
            "srec: unsupported int width (only 4 is implemented)",
        ));
    }

    // Per `reference/mili-python/src/mili/afileIO.py:409-419`,
    // MODIFIER1 is the total int-word count (including the 4-int
    // header) and MODIFIER2 is the char-stream byte length.
    let qty_int_words = i32::try_from(entry.modifier1).map_err(|_| {
        MiliError::MalformedDirectory("srec: MODIFIER1 (int words) out of i32 range")
    })?;
    let qty_char_bytes = i32::try_from(entry.modifier2).map_err(|_| {
        MiliError::MalformedDirectory("srec: MODIFIER2 (char bytes) out of i32 range")
    })?;
    if qty_int_words < 4 {
        return Err(MiliError::MalformedDirectory(
            "srec: qty_int_words < 4 (header alone is 4 ints)",
        ));
    }
    if qty_char_bytes < 0 {
        return Err(MiliError::MalformedDirectory(
            "srec: qty_char_bytes negative",
        ));
    }

    let header_bytes = 4 * int_w;
    let int_payload_bytes = ((qty_int_words - 4) as usize) * int_w;
    let chars_start = header_bytes + int_payload_bytes;
    let chars_end =
        chars_start
            .checked_add(qty_char_bytes as usize)
            .ok_or(MiliError::MalformedDirectory(
                "srec: char-stream end overflow",
            ))?;
    if raw.len() < chars_end {
        return Err(MiliError::MalformedDirectory(
            "srec: payload shorter than int+char streams",
        ));
    }

    let srec_id = end.read_i32(slice4(&raw[0..])?);
    let mesh_id = end.read_i32(slice4(&raw[int_w..])?);
    let srec_size = end.read_i32(slice4(&raw[2 * int_w..])?);
    let qty_subrecs = end.read_i32(slice4(&raw[3 * int_w..])?);
    if qty_subrecs < 0 {
        return Err(MiliError::MalformedDirectory("srec: qty_subrecs < 0"));
    }

    let int_bytes = &raw[header_bytes..chars_start];
    let char_bytes = &raw[chars_start..chars_end];

    let int_data = decode_int_stream(end, int_bytes, int_w)?;
    let strings = decode_char_stream(char_bytes)?;

    let mut iidx = 0usize;
    let mut sidx = 0usize;
    let mut subrecords = Vec::with_capacity(qty_subrecs as usize);
    for _ in 0..qty_subrecs {
        subrecords.push(parse_one_subrecord(
            &int_data, &mut iidx, &strings, &mut sidx,
        )?);
    }

    Ok(Srec {
        srec_id,
        mesh_id,
        srec_size,
        subrecords,
    })
}

fn parse_one_subrecord(
    int_data: &[i32],
    iidx: &mut usize,
    strings: &[String],
    sidx: &mut usize,
) -> Result<Subrecord> {
    let org_code = *int_data
        .get(*iidx)
        .ok_or(MiliError::MalformedDirectory("srec: int stream truncated"))?;
    let qty_svars = *int_data
        .get(*iidx + 1)
        .ok_or(MiliError::MalformedDirectory("srec: missing qty_svars"))?;
    let block_count = *int_data
        .get(*iidx + 2)
        .ok_or(MiliError::MalformedDirectory("srec: missing block_count"))?;
    *iidx += 3;
    let organization = Organization::from_code(org_code)
        .ok_or(MiliError::MalformedDirectory("srec: unknown organization"))?;
    if qty_svars < 0 {
        return Err(MiliError::MalformedDirectory("srec: qty_svars < 0"));
    }
    if block_count < 0 {
        return Err(MiliError::MalformedDirectory("srec: block_count < 0"));
    }
    let qty_svars = qty_svars as usize;
    let block_count = block_count as usize;

    let name = strings
        .get(*sidx)
        .ok_or(MiliError::MalformedDirectory(
            "srec: subrec name past char stream",
        ))?
        .clone();
    let mclass = strings
        .get(*sidx + 1)
        .ok_or(MiliError::MalformedDirectory(
            "srec: mclass name past char stream",
        ))?
        .clone();
    let svars_end = (*sidx)
        .checked_add(2 + qty_svars)
        .ok_or(MiliError::MalformedDirectory(
            "srec: subrec name list overflow",
        ))?;
    if svars_end > strings.len() {
        return Err(MiliError::MalformedDirectory(
            "srec: svar name list past char stream",
        ));
    }
    let svar_names: Vec<String> = strings[*sidx + 2..*sidx + 2 + qty_svars].to_vec();
    *sidx = svars_end;

    let pairs_end = (*iidx)
        .checked_add(2 * block_count)
        .ok_or(MiliError::MalformedDirectory(
            "srec: id-block list overflow",
        ))?;
    if pairs_end > int_data.len() {
        return Err(MiliError::MalformedDirectory(
            "srec: id-block list past int stream",
        ));
    }
    let mut id_blocks = Vec::with_capacity(block_count);
    for k in 0..block_count {
        let s = int_data[*iidx + 2 * k];
        let e = int_data[*iidx + 2 * k + 1];
        id_blocks.push((s, e));
    }
    *iidx = pairs_end;

    Ok(Subrecord {
        name,
        mclass,
        organization,
        svar_names,
        id_blocks,
    })
}

fn decode_int_stream(end: Endianness, bytes: &[u8], int_w: usize) -> Result<Vec<i32>> {
    if !bytes.len().is_multiple_of(int_w) {
        return Err(MiliError::MalformedDirectory(
            "srec: int stream length not a multiple of int width",
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / int_w);
    for chunk in bytes.chunks_exact(int_w) {
        let slot: [u8; 4] = chunk.try_into().expect("chunks_exact(4)");
        out.push(end.read_i32(&slot));
    }
    Ok(out)
}

fn decode_char_stream(bytes: &[u8]) -> Result<Vec<String>> {
    let s = std::str::from_utf8(bytes)
        .map_err(|_| MiliError::MalformedDirectory("srec: char stream not valid UTF-8"))?;
    Ok(s.split('\0')
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect())
}

fn payload<'a>(bytes: &'a [u8], entry: &DirEntry) -> Result<&'a [u8]> {
    let off = usize::try_from(entry.offset)
        .map_err(|_| MiliError::MalformedDirectory("srec: negative offset"))?;
    let len = usize::try_from(entry.length)
        .map_err(|_| MiliError::MalformedDirectory("srec: negative length"))?;
    let end = off.checked_add(len).ok_or(MiliError::MalformedDirectory(
        "srec: offset+length overflow",
    ))?;
    bytes
        .get(off..end)
        .ok_or(MiliError::MalformedDirectory("srec: payload past EOF"))
}

fn slice4(bytes: &[u8]) -> Result<&[u8; 4]> {
    bytes
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .ok_or(MiliError::MalformedDirectory("srec: short int slice"))
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

    // ---------------------------- derive_lumps ----------------------------

    #[test]
    fn lumps_single_scalar_svar() {
        // One scalar f32 svar: atoms=1, width=4.
        let l = derive_lumps(&[1], &[4]);
        assert_eq!(l.atoms, vec![1]);
        assert_eq!(l.sizes, vec![4]);
        assert_eq!(l.offsets, vec![0]);
        assert_eq!(l.bytes_per_object(), 4);
    }

    #[test]
    fn lumps_multiple_scalar_svars() {
        // Three scalar svars with mixed widths: f32, f64, i32.
        let l = derive_lumps(&[1, 1, 1], &[4, 8, 4]);
        assert_eq!(l.sizes, vec![4, 8, 4]);
        assert_eq!(l.offsets, vec![0, 4, 12]);
        assert_eq!(l.bytes_per_object(), 16);
    }

    #[test]
    fn lumps_vector_svar() {
        // One vector svar of 6 components, each 4 bytes wide.
        let l = derive_lumps(&[6], &[4]);
        assert_eq!(l.atoms, vec![6]);
        assert_eq!(l.sizes, vec![24]);
        assert_eq!(l.offsets, vec![0]);
    }

    #[test]
    fn lumps_array_svar() {
        // ARRAY: atoms = prod(dims). Here dims=[2,3]→6 atoms.
        let l = derive_lumps(&[6], &[8]);
        assert_eq!(l.sizes, vec![48]);
    }

    #[test]
    fn lumps_vec_array_svar() {
        // VEC_ARRAY: atoms = prod(dims) * list_size = 3 * 6 = 18.
        let l = derive_lumps(&[18], &[4]);
        assert_eq!(l.sizes, vec![72]);
    }

    #[test]
    fn lumps_mixed_widths_vec_array_plus_scalar() {
        // Mirrors `test_bugfixes.py:119-172`: a vec_array of stress
        // (6 components, f32) plus a scalar eps (1 atom, f32). The
        // outer svar lump captures both per-object lumps; the query
        // layer uses the offsets to walk into the component-level
        // bytes.
        let l = derive_lumps(&[6, 1], &[4, 4]);
        assert_eq!(l.sizes, vec![24, 4]);
        assert_eq!(l.offsets, vec![0, 24]);
        assert_eq!(l.bytes_per_object(), 28);

        // RESULT_ORDERED address for svar 0, obj 0: 0; svar 1, obj 5
        // (over N=10 objects): N*offsets[1] + 5*sizes[1] = 10*24 +
        // 5*4 = 260.
        let n: usize = 10;
        assert_eq!(n * l.offsets[1] + 5 * l.sizes[1], 260);

        // OBJECT_ORDERED address for svar 1, obj 3: 3*stride +
        // offsets[1] = 3*28 + 24 = 108.
        let stride = l.bytes_per_object();
        assert_eq!(3 * stride + l.offsets[1], 108);
    }

    // ---------------------------- srec parser -----------------------------

    /// Test-only subrecord builder spec:
    /// `(organization, subrec_name, mclass, svar_names, id_blocks)`.
    type SubrecSpec<'a> = (i32, &'a str, &'a str, &'a [&'a str], &'a [(i32, i32)]);

    fn build_srec_payload(
        srec_id: i32,
        mesh_id: i32,
        srec_size: i32,
        subrecs: &[SubrecSpec],
    ) -> (Vec<u8>, i64, i64) {
        let mut ints: Vec<i32> = Vec::new();
        let mut chars: Vec<u8> = Vec::new();
        for (org, sname, mclass, svars, blocks) in subrecs {
            ints.push(*org);
            ints.push(svars.len() as i32);
            ints.push(blocks.len() as i32);
            for (s, e) in *blocks {
                ints.push(*s);
                ints.push(*e);
            }
            for s in [sname, mclass] {
                chars.extend_from_slice(s.as_bytes());
                chars.push(0);
            }
            for s in *svars {
                chars.extend_from_slice(s.as_bytes());
                chars.push(0);
            }
        }
        while !chars.len().is_multiple_of(4) {
            chars.push(0);
        }

        let qty_int_words = 4 + ints.len() as i32;
        let qty_char_bytes = chars.len() as i32;

        let mut payload = Vec::new();
        payload.extend_from_slice(&srec_id.to_le_bytes());
        payload.extend_from_slice(&mesh_id.to_le_bytes());
        payload.extend_from_slice(&srec_size.to_le_bytes());
        payload.extend_from_slice(&(subrecs.len() as i32).to_le_bytes());
        for i in &ints {
            payload.extend_from_slice(&i.to_le_bytes());
        }
        payload.extend_from_slice(&chars);
        (payload, qty_int_words as i64, qty_char_bytes as i64)
    }

    fn entry_for(offset: i64, length: i64, modifier1: i64, modifier2: i64) -> DirEntry {
        DirEntry {
            entry_type: DirEntryType::StateRecData,
            modifier1,
            modifier2,
            string_qty: 0,
            offset,
            length,
            name_start: 0,
            name_count: 0,
        }
    }

    #[test]
    fn parses_single_subrecord_result_ordered() {
        let (payload, qint, qchar) = build_srec_payload(
            0,
            0,
            64,
            &[(
                Organization::ResultOrdered as i32,
                "brick_stress",
                "brick",
                &["stress"],
                &[(1, 8)],
            )],
        );
        let mut bytes = vec![0u8; 16];
        bytes.extend_from_slice(&payload);
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![entry_for(16, payload.len() as i64, qint, qchar)],
            names: NamePool::parse(b"", 0).unwrap(),
        };
        let table = SrecTable::build(&bytes, &dir, h()).unwrap();
        assert_eq!(table.len(), 1);
        let srec = table.get(0).unwrap();
        assert_eq!(srec.mesh_id, 0);
        assert_eq!(srec.srec_size, 64);
        assert_eq!(srec.subrecords.len(), 1);
        let sub = &srec.subrecords[0];
        assert_eq!(sub.name, "brick_stress");
        assert_eq!(sub.mclass, "brick");
        assert_eq!(sub.organization, Organization::ResultOrdered);
        assert_eq!(sub.svar_names, vec!["stress".to_owned()]);
        assert_eq!(sub.id_blocks, vec![(1, 8)]);
        assert_eq!(sub.object_count(), 8);
    }

    #[test]
    fn parses_multi_subrecord_with_mixed_organization() {
        let (payload, qint, qchar) = build_srec_payload(
            2,
            0,
            128,
            &[
                (
                    Organization::ResultOrdered as i32,
                    "a",
                    "brick",
                    &["sx", "sy"],
                    &[(1, 2)],
                ),
                (
                    Organization::ObjectOrdered as i32,
                    "b",
                    "shell",
                    &["eps"],
                    &[(10, 12), (20, 22)],
                ),
            ],
        );
        let mut bytes = vec![0u8; 16];
        bytes.extend_from_slice(&payload);
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![entry_for(16, payload.len() as i64, qint, qchar)],
            names: NamePool::parse(b"", 0).unwrap(),
        };
        let table = SrecTable::build(&bytes, &dir, h()).unwrap();
        let srec = table.get(2).unwrap();
        assert_eq!(srec.subrecords.len(), 2);
        assert_eq!(srec.subrecords[0].organization, Organization::ResultOrdered);
        assert_eq!(srec.subrecords[1].organization, Organization::ObjectOrdered);
        assert_eq!(srec.subrecords[1].id_blocks, vec![(10, 12), (20, 22)]);
        assert_eq!(srec.subrecords[1].object_count(), 6);
    }

    #[test]
    fn rejects_unknown_organization() {
        let (payload, qint, qchar) =
            build_srec_payload(0, 0, 32, &[(99, "x", "brick", &["sx"], &[(1, 1)])]);
        let mut bytes = vec![0u8; 16];
        bytes.extend_from_slice(&payload);
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![entry_for(16, payload.len() as i64, qint, qchar)],
            names: NamePool::parse(b"", 0).unwrap(),
        };
        assert!(SrecTable::build(&bytes, &dir, h()).is_err());
    }
}
