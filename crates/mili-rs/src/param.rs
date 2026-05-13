//! Decode `MILI_PARAM` / `APPLICATION_PARAM` / `TI_PARAM` payloads.
//!
//! All three entry types share an identical on-disk encoding; only the
//! `TYPE` field in the directory entry distinguishes them
//! (`reference/mili/src/param.c:101-333, 385-650, 752-941`). The
//! distinction matters semantically — `MILI_PARAM` is internal mili
//! bookkeeping, `APPLICATION_PARAM` is caller-supplied, `TI_PARAM` is
//! "time-independent" — but the byte layout is the same:
//!
//! - `MODIFIER1` carries the [`DataType`] code (M_INT, M_FLOAT, M_STRING, …).
//! - `MODIFIER2` carries the [`AggType`] code (SCALAR, ARRAY) or
//!   `DONT_CARE`(0) for strings.
//! - `OFFSET .. OFFSET+LENGTH` covers the payload in the owning file.
//!
//! See `planning/shared/entry-payloads.md` § "MILI_PARAM, APPLICATION_PARAM,
//! TI_PARAM" for the byte schemas.
//!
//! In directory v2+ databases (every fixture we ship except the
//! deferred v1 case), `TI_PARAM` entries live inline in the main `.A`
//! directory and are looked up against the same name table as the
//! other params — `reference/mili/src/direc.c:653-689` and
//! `reference/mili/src/ti.c:179-212` (the latter short-circuits to
//! `mc_read_scalar` whenever `DIR_VERSION_IDX > 1`). For v1 only,
//! TI_PARAMs live in a separate `<root>_TI_A…` file; that path is
//! deferred along with v1 directory support and stubbed in [`crate::ti`].

use std::collections::HashMap;

use crate::directory::{DirEntry, DirEntryType, Directory};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};

/// On-disk numeric type codes from `reference/mili/src/mili.h:54-60`.
///
/// `Float` and `Int` are width-aliases resolved by the header's
/// precision-limit byte; per the resolved question both currently
/// resolve to 4-byte widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum DataType {
    String = 1,
    Float = 2,
    Float4 = 3,
    Float8 = 4,
    Int = 5,
    Int4 = 6,
    Int8 = 7,
}

impl DataType {
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1 => Self::String,
            2 => Self::Float,
            3 => Self::Float4,
            4 => Self::Float8,
            5 => Self::Int,
            6 => Self::Int4,
            7 => Self::Int8,
            _ => return None,
        })
    }

    /// On-disk width in bytes, resolving the `M_FLOAT` / `M_INT` aliases
    /// against the header's precision-limit byte. See
    /// `planning/shared/format.md` § "Numeric types".
    pub fn width(self, header: &Header) -> usize {
        match self {
            Self::String => 1,
            Self::Float4 | Self::Int4 => 4,
            Self::Float8 | Self::Int8 => 8,
            Self::Float => header.float_size(),
            Self::Int => header.int_size(),
        }
    }
}

/// Aggregate-type codes (`reference/mili/src/mili.h:160-167`). Only
/// `Scalar` and `Array` appear in param entries; the other two
/// (`Vector`, `VecArray`) are svar-specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum AggType {
    Scalar = 0,
    Vector = 1,
    Array = 2,
    VecArray = 3,
}

impl AggType {
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Scalar,
            1 => Self::Vector,
            2 => Self::Array,
            3 => Self::VecArray,
            _ => return None,
        })
    }
}

/// Decoded scalar payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalarValue {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

/// Decoded array payload. `data` is a slice into the parent file's
/// bytes; widen / byteswap is the caller's job once `MiliBuffer` lands.
#[derive(Debug)]
pub struct ArrayParam<'a> {
    pub data_type: DataType,
    pub dims: Vec<i32>,
    pub atoms: usize,
    pub data: &'a [u8],
}

/// Decoded param payload, borrowed from the parent file's byte slice.
#[derive(Debug)]
pub enum ParamValue<'a> {
    Scalar(ScalarValue),
    String(&'a str),
    Array(ArrayParam<'a>),
}

impl<'a> ParamValue<'a> {
    /// Decode a single param entry against its parent file's bytes.
    ///
    /// `entry` must be one of [`DirEntryType::MiliParam`],
    /// [`DirEntryType::ApplicationParam`], or [`DirEntryType::TiParam`].
    pub fn decode(bytes: &'a [u8], entry: &DirEntry, header: Header) -> Result<Self> {
        match entry.entry_type {
            DirEntryType::MiliParam | DirEntryType::ApplicationParam | DirEntryType::TiParam => {}
            t => {
                return Err(MiliError::MalformedDirectory(match t {
                    DirEntryType::Nodes => "decode called on Nodes entry",
                    DirEntryType::ElemConns => "decode called on ElemConns entry",
                    DirEntryType::ClassIdents => "decode called on ClassIdents entry",
                    DirEntryType::StateVarDict => "decode called on StateVarDict entry",
                    DirEntryType::StateRecData => "decode called on StateRecData entry",
                    DirEntryType::ClassDef => "decode called on ClassDef entry",
                    DirEntryType::SurfaceConns => "decode called on SurfaceConns entry",
                    _ => "decode called on non-param entry",
                }));
            }
        }

        let data_type = DataType::from_code(entry.modifier1)
            .ok_or(MiliError::MalformedDirectory("param: unknown data type"))?;
        let agg = AggType::from_code(entry.modifier2)
            .ok_or(MiliError::MalformedDirectory("param: unknown agg type"))?;

        // C dispatch (`param.c:1198, 1210, 1379`): MODIFIER2 == ARRAY → array,
        // else if MODIFIER1 == M_STRING → string, else scalar. SCALAR (0)
        // and DONT_CARE (0) overlap by design.
        if agg == AggType::Array {
            return Ok(Self::Array(decode_array(bytes, entry, header, data_type)?));
        }
        if data_type == DataType::String {
            return Ok(Self::String(decode_string(bytes, entry)?));
        }
        if agg == AggType::Scalar {
            return Ok(Self::Scalar(decode_scalar(
                bytes, entry, header, data_type,
            )?));
        }
        Err(MiliError::MalformedDirectory(
            "param: unsupported aggregate (vector / vec_array not valid here)",
        ))
    }
}

fn payload<'a>(bytes: &'a [u8], entry: &DirEntry) -> Result<&'a [u8]> {
    let off = usize::try_from(entry.offset)
        .map_err(|_| MiliError::MalformedDirectory("param: negative offset"))?;
    let len = usize::try_from(entry.length)
        .map_err(|_| MiliError::MalformedDirectory("param: negative length"))?;
    let end = off.checked_add(len).ok_or(MiliError::MalformedDirectory(
        "param: offset+length overflow",
    ))?;
    bytes
        .get(off..end)
        .ok_or(MiliError::MalformedDirectory("param: payload past EOF"))
}

fn decode_string<'a>(bytes: &'a [u8], entry: &DirEntry) -> Result<&'a str> {
    let raw = payload(bytes, entry)?;
    // The writer pads strings up to an 8-byte boundary with trailing
    // NULs (`param.c:530`). Trim back to the first NUL — the C reader
    // hands back the buffer raw, and the Python lib slices on the NUL.
    let nul = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    std::str::from_utf8(&raw[..nul])
        .map_err(|_| MiliError::MalformedDirectory("param string: bad UTF-8"))
}

fn decode_scalar(
    bytes: &[u8],
    entry: &DirEntry,
    header: Header,
    data_type: DataType,
) -> Result<ScalarValue> {
    let raw = payload(bytes, entry)?;
    let width = data_type.width(&header);
    if raw.len() < width {
        return Err(MiliError::MalformedDirectory(
            "param scalar: payload shorter than width",
        ));
    }
    let end = header.endianness;
    let slot = &raw[..width];
    Ok(match data_type {
        DataType::Int | DataType::Int4 => {
            ScalarValue::I32(end.read_i32(slot.try_into().expect("4 bytes")))
        }
        DataType::Int8 => ScalarValue::I64(end.read_i64(slot.try_into().expect("8 bytes"))),
        DataType::Float | DataType::Float4 => {
            ScalarValue::F32(read_f32(end, *<&[u8; 4]>::try_from(slot).expect("4 bytes")))
        }
        DataType::Float8 => {
            ScalarValue::F64(read_f64(end, *<&[u8; 8]>::try_from(slot).expect("8 bytes")))
        }
        DataType::String => {
            return Err(MiliError::MalformedDirectory(
                "string param routed to scalar decoder",
            ));
        }
    })
}

fn decode_array<'a>(
    bytes: &'a [u8],
    entry: &DirEntry,
    header: Header,
    data_type: DataType,
) -> Result<ArrayParam<'a>> {
    let raw = payload(bytes, entry)?;
    let int_w = header.int_size();
    let end = header.endianness;
    // rank, then `rank` dims, then the data — all using M_INT for the
    // header words (`param.c:793-817`).
    let rank_slot = raw
        .get(..int_w)
        .ok_or(MiliError::MalformedDirectory("array param: rank short"))?;
    let rank = read_int(end, rank_slot, int_w)?;
    if rank < 0 {
        return Err(MiliError::MalformedDirectory("array param: rank < 0"));
    }
    let rank_us = usize::try_from(rank)
        .map_err(|_| MiliError::MalformedDirectory("array param: rank too large"))?;

    let dims_bytes = int_w
        .checked_mul(rank_us)
        .ok_or(MiliError::MalformedDirectory("array param: dims overflow"))?;
    let header_bytes = int_w
        .checked_add(dims_bytes)
        .ok_or(MiliError::MalformedDirectory(
            "array param: header overflow",
        ))?;
    if raw.len() < header_bytes {
        return Err(MiliError::MalformedDirectory(
            "array param: header past payload end",
        ));
    }

    let mut dims = Vec::with_capacity(rank_us);
    let mut atoms: usize = 1;
    for i in 0..rank_us {
        let off = int_w + i * int_w;
        let d = read_int(end, &raw[off..off + int_w], int_w)?;
        if d < 0 {
            return Err(MiliError::MalformedDirectory("array param: negative dim"));
        }
        let du = usize::try_from(d)
            .map_err(|_| MiliError::MalformedDirectory("array param: dim too large"))?;
        let di32 = i32::try_from(d)
            .map_err(|_| MiliError::MalformedDirectory("array param: dim exceeds i32"))?;
        atoms = atoms.checked_mul(du).ok_or(MiliError::MalformedDirectory(
            "array param: atom count overflow",
        ))?;
        dims.push(di32);
    }

    let elem_w = data_type.width(&header);
    let body_bytes = atoms
        .checked_mul(elem_w)
        .ok_or(MiliError::MalformedDirectory("array param: body overflow"))?;
    let body_end = header_bytes
        .checked_add(body_bytes)
        .ok_or(MiliError::MalformedDirectory("array param: end overflow"))?;
    if raw.len() < body_end {
        return Err(MiliError::MalformedDirectory(
            "array param: body past payload end",
        ));
    }
    let data = &raw[header_bytes..body_end];
    Ok(ArrayParam {
        data_type,
        dims,
        atoms,
        data,
    })
}

fn read_int(end: Endianness, slot: &[u8], width: usize) -> Result<i64> {
    match width {
        4 => Ok(i64::from(end.read_i32(slot.try_into().map_err(|_| {
            MiliError::MalformedDirectory("read_int: bad slice")
        })?))),
        8 => Ok(end.read_i64(
            slot.try_into()
                .map_err(|_| MiliError::MalformedDirectory("read_int: bad slice"))?,
        )),
        _ => Err(MiliError::MalformedDirectory("read_int: unsupported width")),
    }
}

fn read_f32(end: Endianness, slot: [u8; 4]) -> f32 {
    match end {
        Endianness::Big => f32::from_be_bytes(slot),
        Endianness::Little => f32::from_le_bytes(slot),
    }
}

fn read_f64(end: Endianness, slot: [u8; 8]) -> f64 {
    match end {
        Endianness::Big => f64::from_be_bytes(slot),
        Endianness::Little => f64::from_le_bytes(slot),
    }
}

/// Name → directory-entry-index map for the three param entry types.
///
/// The C library stores every param in a single hash table keyed by
/// name (`direc.c:653-689`). We do the same: in v2+ databases the
/// table covers `MILI_PARAM`, `APPLICATION_PARAM`, and `TI_PARAM`
/// entries from the same main `.A` directory. For v1 databases TI
/// params live in a separate file and would be indexed by a parallel
/// table on the v1 path — out of scope here.
///
/// When multiple entries share a name (the writer appends rather than
/// overwriting; `read_param_array_len` walks the chain at
/// `param.c:728-743`), [`Self::all`] returns every match in directory
/// order. [`Self::get`] returns the first.
#[derive(Debug, Default)]
pub struct ParamTable {
    by_name: HashMap<String, Vec<usize>>,
}

impl ParamTable {
    /// Build the index from a parsed directory. Only entries of param
    /// type are recorded; everything else is skipped.
    pub fn build(dir: &Directory) -> Self {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, entry) in dir.entries.iter().enumerate() {
            if !matches!(
                entry.entry_type,
                DirEntryType::MiliParam | DirEntryType::ApplicationParam | DirEntryType::TiParam
            ) {
                continue;
            }
            if entry.name_count == 0 {
                continue;
            }
            let name = dir.names.get(entry.name_start as usize).to_owned();
            by_name.entry(name).or_default().push(idx);
        }
        Self { by_name }
    }

    pub fn len(&self) -> usize {
        self.by_name.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).and_then(|v| v.first().copied())
    }

    pub fn all(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Filter param names by a string predicate. Used by the
    /// TI_PARAM-as-storage accessors (labels, materials, element sets);
    /// see `planning/shared/format.md` § "TI_PARAM-as-storage pattern".
    pub fn names_matching<'a, F>(&'a self, mut pred: F) -> impl Iterator<Item = &'a str>
    where
        F: FnMut(&str) -> bool + 'a,
    {
        self.by_name.keys().filter_map(move |n| {
            if pred(n.as_str()) {
                Some(n.as_str())
            } else {
                None
            }
        })
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

    #[test]
    fn scalar_i32_round_trip() {
        let mut bytes = vec![0u8; 32];
        bytes[16..20].copy_from_slice(&42i32.to_le_bytes());
        let e = entry(
            DirEntryType::MiliParam,
            DataType::Int as i64,
            0,
            16,
            4,
            0,
            1,
        );
        let v = ParamValue::decode(&bytes, &e, h()).unwrap();
        assert!(matches!(v, ParamValue::Scalar(ScalarValue::I32(42))));
    }

    #[test]
    fn scalar_f64_round_trip() {
        let mut bytes = vec![0u8; 32];
        bytes[8..16].copy_from_slice(&3.5f64.to_le_bytes());
        let e = entry(
            DirEntryType::MiliParam,
            DataType::Float8 as i64,
            AggType::Scalar as i64,
            8,
            8,
            0,
            1,
        );
        let v = ParamValue::decode(&bytes, &e, h()).unwrap();
        match v {
            ParamValue::Scalar(ScalarValue::F64(f)) => assert!((f - 3.5).abs() < 1e-12),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn string_trims_trailing_nuls() {
        let mut bytes = vec![0u8; 32];
        // "hi\0\0\0\0\0\0" → length 8, content "hi"
        bytes[16] = b'h';
        bytes[17] = b'i';
        let e = entry(
            DirEntryType::ApplicationParam,
            DataType::String as i64,
            0,
            16,
            8,
            0,
            1,
        );
        match ParamValue::decode(&bytes, &e, h()).unwrap() {
            ParamValue::String(s) => assert_eq!(s, "hi"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn array_int_round_trip() {
        // rank=1, dims=[3], data=[7, 8, 9] (int32)
        let mut bytes = vec![0u8; 64];
        let mut cur = 16;
        for w in [1i32, 3, 7, 8, 9] {
            bytes[cur..cur + 4].copy_from_slice(&w.to_le_bytes());
            cur += 4;
        }
        let len = (cur - 16) as i64;
        let e = entry(
            DirEntryType::TiParam,
            DataType::Int as i64,
            AggType::Array as i64,
            16,
            len,
            0,
            1,
        );
        match ParamValue::decode(&bytes, &e, h()).unwrap() {
            ParamValue::Array(a) => {
                assert_eq!(a.dims, vec![3]);
                assert_eq!(a.atoms, 3);
                assert_eq!(a.data.len(), 12);
                let v: Vec<i32> = a
                    .data
                    .chunks(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                assert_eq!(v, vec![7, 8, 9]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn array_rejects_oob_payload() {
        // rank=1, dims=[10], but length only covers rank+dims, no body.
        let mut bytes = vec![0u8; 32];
        bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
        bytes[20..24].copy_from_slice(&10i32.to_le_bytes());
        let e = entry(
            DirEntryType::MiliParam,
            DataType::Int as i64,
            AggType::Array as i64,
            16,
            8,
            0,
            1,
        );
        assert!(ParamValue::decode(&bytes, &e, h()).is_err());
    }

    #[test]
    fn decode_rejects_non_param_entry() {
        let e = entry(DirEntryType::Nodes, 0, 0, 0, 0, 0, 1);
        assert!(ParamValue::decode(&[], &e, h()).is_err());
    }

    #[test]
    fn param_table_indexes_only_param_entries() {
        // Build a directory with a Nodes entry and two TiParam entries
        // sharing a name. Verify all/get behavior.
        use crate::directory::Directory;
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![
                entry(DirEntryType::Nodes, 0, 0, 0, 0, 0, 1),
                entry(DirEntryType::TiParam, 5, 0, 0, 4, 1, 1),
                entry(DirEntryType::TiParam, 5, 0, 0, 4, 2, 1),
            ],
            names: NamePool::parse(b"node\0count\0count\0", 3).unwrap(),
        };
        let table = ParamTable::build(&dir);
        assert_eq!(table.len(), 2);
        assert_eq!(table.get("node"), None);
        assert_eq!(table.get("count"), Some(1));
        assert_eq!(table.all("count"), &[1, 2]);
    }
}
