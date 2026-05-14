//! `STATE_VAR_DICT` payload decoding.
//!
//! The svar dictionary is a single payload built from two streams laid
//! out consecutively: an integer stream and a character stream. The
//! parser walks both in lockstep. See
//! `planning/shared/entry-payloads.md` § `STATE_VAR_DICT` for the byte
//! layout.
//!
//! Each svar resolves to a [`Svar`] carrying its name, title, numeric
//! type, aggregate kind, and the precomputed total atom count (atoms
//! per object — the per-svar entry in the byte-layout matrix in
//! `planning/shared/format.md` § "Subrecord byte-layout matrix").
//!
//! Component names of vector and vec-array svars live **inside** the
//! character stream rather than in the file-level name pool. When a
//! component name has not yet been registered in the table, the parser
//! recurses and consumes the next svar definition from the same int /
//! char streams (`reference/mili-python/src/mili/afileIO.py:330-352`).

use std::collections::HashMap;

use crate::directory::{DirEntry, DirEntryType, Directory};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};

/// On-disk numeric type codes for svar data (`reference/mili/src/mili.h:54-60`).
///
/// Unlike the param-level [`crate::param::DataType`], svar `num_type`
/// fields are always one of the four width-explicit variants — the
/// platform-native `M_FLOAT` / `M_INT` aliases are not used here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumType {
    Int4,
    Int8,
    Float4,
    Float8,
}

impl NumType {
    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            // M_FLOAT / M_INT aliases collapse to their 4-byte
            // resolution per the resolved-question in
            // `planning/shared/format.md` § "Numeric types".
            2 | 3 => Self::Float4,
            4 => Self::Float8,
            5 | 6 => Self::Int4,
            7 => Self::Int8,
            _ => return None,
        })
    }

    pub fn width(self) -> usize {
        match self {
            Self::Int4 | Self::Float4 => 4,
            Self::Int8 | Self::Float8 => 8,
        }
    }
}

/// Aggregate-type kinds for svars (`mili_enum.h` agg codes 0..=3).
///
/// `Scalar` carries no extra metadata. `Array` carries `dims` (rank =
/// `dims.len()`). `Vector` and `VecArray` carry the embedded component
/// names — their order in the on-disk stream is the order in the
/// byte-layout matrix in `planning/shared/format.md` §
/// "Subrecord byte-layout matrix".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvarAgg {
    Scalar,
    Vector { comps: Vec<String> },
    Array { dims: Vec<i32> },
    VecArray { dims: Vec<i32>, comps: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct Svar {
    pub name: String,
    pub title: String,
    pub num_type: NumType,
    pub agg: SvarAgg,
    /// Atoms-per-object for this svar, resolved at parse time:
    /// `1` for scalar, `comps.len()` for vector, `prod(dims)` for
    /// array, `prod(dims) * comps.len()` for vec-array. This is the
    /// per-object cell in the byte-layout matrix; the byte width per
    /// object is `atoms * num_type.width()`.
    pub atoms: usize,
}

/// Name-keyed lookup over every svar parsed from the directory's
/// `STATE_VAR_DICT` entries.
#[derive(Debug, Default)]
pub struct SvarTable {
    by_name: HashMap<String, Svar>,
    order: Vec<String>,
}

impl SvarTable {
    /// Build the table by walking every `STATE_VAR_DICT` entry in
    /// directory order.
    pub fn build(bytes: &[u8], dir: &Directory, header: Header) -> Result<Self> {
        let mut table = SvarTable::default();
        for entry in &dir.entries {
            if entry.entry_type == DirEntryType::StateVarDict {
                parse_dict_entry(bytes, entry, header, &mut table)?;
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

    pub fn get(&self, name: &str) -> Option<&Svar> {
        self.by_name.get(name)
    }

    /// Iterate svars in the order they were parsed from the directory.
    pub fn iter(&self) -> impl Iterator<Item = &Svar> {
        self.order
            .iter()
            .map(|n| self.by_name.get(n).expect("order index consistent"))
    }

    /// Names in parse order. Matches what mili-python's
    /// `state_variables().keys()` returns
    /// (`reference/mili-python/src/mili/afileIO.py:315-352`).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }
}

fn parse_dict_entry(
    bytes: &[u8],
    entry: &DirEntry,
    header: Header,
    table: &mut SvarTable,
) -> Result<()> {
    let raw = payload(bytes, entry)?;
    let int_w = header.int_size();
    let end = header.endianness;

    if raw.len() < 2 * int_w {
        return Err(MiliError::MalformedDirectory(
            "STATE_VAR_DICT: payload shorter than 2-int header",
        ));
    }
    // SVAR_QTY_INT_WORDS is the total int-word count in the int
    // stream, *including* the two header words. SVAR_QTY_BYTES is the
    // char-stream length (`reference/mili-python/src/mili/afileIO.py:317-322`).
    let qty_int_words = read_i32(end, &raw[..int_w], int_w)?;
    let qty_char_bytes = read_i32(end, &raw[int_w..2 * int_w], int_w)?;
    if qty_int_words < 2 {
        return Err(MiliError::MalformedDirectory(
            "STATE_VAR_DICT: qty_int_words < 2",
        ));
    }
    if qty_char_bytes < 0 {
        return Err(MiliError::MalformedDirectory(
            "STATE_VAR_DICT: qty_char_bytes < 0",
        ));
    }
    let int_payload_bytes = ((qty_int_words - 2) as usize) * int_w;
    let chars_start = 2 * int_w + int_payload_bytes;
    let chars_end =
        chars_start
            .checked_add(qty_char_bytes as usize)
            .ok_or(MiliError::MalformedDirectory(
                "STATE_VAR_DICT: char-stream end overflow",
            ))?;
    if raw.len() < chars_end {
        return Err(MiliError::MalformedDirectory(
            "STATE_VAR_DICT: payload shorter than int+char streams",
        ));
    }

    let int_bytes = &raw[2 * int_w..chars_start];
    let char_bytes = &raw[chars_start..chars_end];

    let int_data = decode_int_stream(end, int_bytes, int_w)?;
    let strings = decode_char_stream(char_bytes)?;

    let mut iidx = 0usize;
    let mut sidx = 0usize;
    while iidx < int_data.len() {
        parse_one(&int_data, &mut iidx, &strings, &mut sidx, table)?;
    }
    Ok(())
}

fn parse_one(
    int_data: &[i32],
    iidx: &mut usize,
    strings: &[String],
    sidx: &mut usize,
    table: &mut SvarTable,
) -> Result<()> {
    let agg_code = *int_data
        .get(*iidx)
        .ok_or(MiliError::MalformedDirectory("svar: int stream truncated"))?;
    let type_code = *int_data
        .get(*iidx + 1)
        .ok_or(MiliError::MalformedDirectory("svar: int stream truncated"))?;
    let name = strings
        .get(*sidx)
        .ok_or(MiliError::MalformedDirectory("svar: char stream truncated"))?
        .clone();
    let title = strings
        .get(*sidx + 1)
        .ok_or(MiliError::MalformedDirectory("svar: char stream truncated"))?
        .clone();
    *iidx += 2;
    *sidx += 2;

    let num_type =
        NumType::from_code(type_code).ok_or(MiliError::MalformedDirectory("svar: bad num_type"))?;

    let dims = if matches!(agg_code, 2 | 3) {
        let order = *int_data
            .get(*iidx)
            .ok_or(MiliError::MalformedDirectory("svar: missing rank"))?;
        if order < 0 {
            return Err(MiliError::MalformedDirectory("svar: negative rank"));
        }
        let order_us = order as usize;
        let dims_end = (*iidx)
            .checked_add(1 + order_us)
            .ok_or(MiliError::MalformedDirectory("svar: rank overflow"))?;
        if dims_end > int_data.len() {
            return Err(MiliError::MalformedDirectory(
                "svar: dims past int stream end",
            ));
        }
        let dims = int_data[*iidx + 1..*iidx + 1 + order_us].to_vec();
        *iidx = dims_end;
        Some(dims)
    } else {
        None
    };

    let comps = if matches!(agg_code, 1 | 3) {
        let list_size = *int_data
            .get(*iidx)
            .ok_or(MiliError::MalformedDirectory("svar: missing list_size"))?;
        if list_size < 0 {
            return Err(MiliError::MalformedDirectory("svar: negative list_size"));
        }
        let list_size_us = list_size as usize;
        *iidx += 1;
        let comps_end = (*sidx)
            .checked_add(list_size_us)
            .ok_or(MiliError::MalformedDirectory("svar: comps overflow"))?;
        if comps_end > strings.len() {
            return Err(MiliError::MalformedDirectory(
                "svar: comp names past char stream end",
            ));
        }
        let comps: Vec<String> = strings[*sidx..*sidx + list_size_us].to_vec();
        *sidx = comps_end;
        Some(comps)
    } else {
        None
    };

    // Recurse to materialise any component svars that haven't been
    // parsed yet — they live inline in the same int/char streams. This
    // mirrors `__parse_svar` in
    // `reference/mili-python/src/mili/afileIO.py:347-350`.
    if let Some(ref c) = comps {
        for comp_name in c {
            if !table.by_name.contains_key(comp_name) {
                parse_one(int_data, iidx, strings, sidx, table)?;
            }
        }
    }

    let agg = match (dims, comps) {
        (None, None) => SvarAgg::Scalar,
        (None, Some(c)) => SvarAgg::Vector { comps: c },
        (Some(d), None) => SvarAgg::Array { dims: d },
        (Some(d), Some(c)) => SvarAgg::VecArray { dims: d, comps: c },
    };

    let atoms = match &agg {
        SvarAgg::Scalar => 1usize,
        SvarAgg::Vector { comps } => comps.len(),
        SvarAgg::Array { dims } => dims_product(dims)?,
        SvarAgg::VecArray { dims, comps } => dims_product(dims)?
            .checked_mul(comps.len())
            .ok_or(MiliError::MalformedDirectory("svar: atom count overflow"))?,
    };

    let svar = Svar {
        name: name.clone(),
        title,
        num_type,
        agg,
        atoms,
    };
    if !table.by_name.contains_key(&name) {
        table.order.push(name.clone());
    }
    table.by_name.insert(name, svar);
    Ok(())
}

fn dims_product(dims: &[i32]) -> Result<usize> {
    let mut acc: usize = 1;
    for &d in dims {
        if d < 0 {
            return Err(MiliError::MalformedDirectory("svar: negative dim"));
        }
        let du = d as usize;
        acc = acc
            .checked_mul(du)
            .ok_or(MiliError::MalformedDirectory("svar: dim product overflow"))?;
    }
    Ok(acc)
}

fn decode_int_stream(end: Endianness, bytes: &[u8], int_w: usize) -> Result<Vec<i32>> {
    if int_w != 4 {
        return Err(MiliError::MalformedDirectory(
            "svar: unsupported int width (only 4 is implemented)",
        ));
    }
    if !bytes.len().is_multiple_of(int_w) {
        return Err(MiliError::MalformedDirectory(
            "svar: int stream length not a multiple of int width",
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
        .map_err(|_| MiliError::MalformedDirectory("svar: char stream not valid UTF-8"))?;
    // The on-disk char stream is a series of NUL-terminated names; the
    // trailing alignment padding shows up as empty splits, which we
    // drop. Matches `reference/mili-python/src/mili/afileIO.py:322-323`.
    Ok(s.split('\0')
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect())
}

fn payload<'a>(bytes: &'a [u8], entry: &DirEntry) -> Result<&'a [u8]> {
    let off = usize::try_from(entry.offset)
        .map_err(|_| MiliError::MalformedDirectory("svar: negative offset"))?;
    let len = usize::try_from(entry.length)
        .map_err(|_| MiliError::MalformedDirectory("svar: negative length"))?;
    let end = off.checked_add(len).ok_or(MiliError::MalformedDirectory(
        "svar: offset+length overflow",
    ))?;
    bytes
        .get(off..end)
        .ok_or(MiliError::MalformedDirectory("svar: payload past EOF"))
}

fn read_i32(end: Endianness, slot: &[u8], width: usize) -> Result<i32> {
    match width {
        4 => {
            let arr: &[u8; 4] = slot
                .try_into()
                .map_err(|_| MiliError::MalformedDirectory("svar: bad 4-byte slice"))?;
            Ok(end.read_i32(arr))
        }
        8 => {
            let arr: &[u8; 8] = slot
                .try_into()
                .map_err(|_| MiliError::MalformedDirectory("svar: bad 8-byte slice"))?;
            let v = end.read_i64(arr);
            i32::try_from(v).map_err(|_| MiliError::MalformedDirectory("svar: int exceeds i32"))
        }
        _ => Err(MiliError::MalformedDirectory("svar: unsupported int width")),
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

    /// Build a synthetic STATE_VAR_DICT payload from int + char streams.
    fn build_dict_payload(ints: &[i32], strings: &[&str]) -> Vec<u8> {
        let qty_int_words = (2 + ints.len()) as i32;
        let mut chars: Vec<u8> = Vec::new();
        for s in strings {
            chars.extend_from_slice(s.as_bytes());
            chars.push(0);
        }
        // Pad to a 4-byte boundary, matching the writer in
        // `reference/mili-python/src/mili/afileIO.py:653-655`.
        while !chars.len().is_multiple_of(4) {
            chars.push(0);
        }
        let qty_char_bytes = chars.len() as i32;

        let mut payload = Vec::new();
        payload.extend_from_slice(&qty_int_words.to_le_bytes());
        payload.extend_from_slice(&qty_char_bytes.to_le_bytes());
        for &i in ints {
            payload.extend_from_slice(&i.to_le_bytes());
        }
        payload.extend_from_slice(&chars);
        payload
    }

    fn entry(offset: i64, length: i64) -> DirEntry {
        DirEntry {
            entry_type: DirEntryType::StateVarDict,
            modifier1: 0,
            modifier2: 0,
            string_qty: 0,
            offset,
            length,
            name_start: 0,
            name_count: 0,
        }
    }

    fn build_dir_with(payload: &[u8]) -> (Vec<u8>, Directory) {
        let mut bytes = vec![0u8; 16];
        bytes.extend_from_slice(payload);
        let entries = vec![entry(16, payload.len() as i64)];
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries,
            names: NamePool::parse(b"", 0).unwrap(),
        };
        (bytes, dir)
    }

    #[test]
    fn parses_scalar_svar() {
        // One scalar svar named "sand" of type M_INT4. Int stream:
        // [agg=0, dtype=6]; char stream: ["sand", "Element Killed"].
        let ints = [0i32, 6];
        let strings = ["sand", "Element Killed"];
        let payload = build_dict_payload(&ints, &strings);
        let (bytes, dir) = build_dir_with(&payload);

        let table = SvarTable::build(&bytes, &dir, h()).unwrap();
        let svar = table.get("sand").expect("sand parsed");
        assert_eq!(svar.title, "Element Killed");
        assert_eq!(svar.num_type, NumType::Int4);
        assert_eq!(svar.agg, SvarAgg::Scalar);
        assert_eq!(svar.atoms, 1);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn parses_array_svar() {
        // One array svar "hx" of M_FLOAT4, rank=1, dims=[8].
        let ints = [2i32, 3, 1, 8];
        let strings = ["hx", "Hex array"];
        let payload = build_dict_payload(&ints, &strings);
        let (bytes, dir) = build_dir_with(&payload);

        let table = SvarTable::build(&bytes, &dir, h()).unwrap();
        let svar = table.get("hx").unwrap();
        assert_eq!(svar.num_type, NumType::Float4);
        match &svar.agg {
            SvarAgg::Array { dims } => assert_eq!(dims, &vec![8]),
            _ => panic!("expected Array"),
        }
        assert_eq!(svar.atoms, 8);
    }

    #[test]
    fn parses_vector_with_inline_component_svars() {
        // Vector svar "stress" of M_FLOAT4 with components ["sx", "sy"].
        // Each component is itself a scalar svar parsed inline via the
        // recursion path. Char-stream order matches python's recursive
        // walk: parent name + title, then comp names, then each comp's
        // own name + title in turn.
        let ints = [1i32, 3, 2, 0, 3, 0, 3];
        let strings = [
            "stress", "Stress", "sx", "sy", "sx", "X Stress", "sy", "Y Stress",
        ];
        let payload = build_dict_payload(&ints, &strings);
        let (bytes, dir) = build_dir_with(&payload);

        let table = SvarTable::build(&bytes, &dir, h()).unwrap();
        let stress = table.get("stress").expect("stress");
        match &stress.agg {
            SvarAgg::Vector { comps } => assert_eq!(comps, &vec!["sx".to_owned(), "sy".to_owned()]),
            _ => panic!("expected Vector"),
        }
        assert_eq!(stress.atoms, 2);
        let sx = table.get("sx").expect("sx parsed recursively");
        assert_eq!(sx.title, "X Stress");
        assert_eq!(sx.agg, SvarAgg::Scalar);
        let sy = table.get("sy").expect("sy parsed recursively");
        assert_eq!(sy.title, "Y Stress");
    }

    #[test]
    fn vector_skips_recursion_when_component_already_parsed() {
        // sx defined first, then stress references it — no recursion
        // needed; the int stream after stress's header should have
        // nothing left to consume.
        let ints = [0i32, 3, /* sx */ 1, 3, 1 /* stress with 1 comp */];
        let strings = ["sx", "X Stress", "stress", "Stress", "sx"];
        let payload = build_dict_payload(&ints, &strings);
        let (bytes, dir) = build_dir_with(&payload);

        let table = SvarTable::build(&bytes, &dir, h()).unwrap();
        let stress = table.get("stress").unwrap();
        match &stress.agg {
            SvarAgg::Vector { comps } => assert_eq!(comps, &vec!["sx".to_owned()]),
            _ => panic!("expected Vector"),
        }
    }

    #[test]
    fn vec_array_atoms_is_dim_product_times_components() {
        // VecArray "stress_arr": rank=1, dims=[3], comps=[sx, sy].
        // atoms = 3 * 2 = 6.
        let ints = [3i32, 3, 1, 3, 2, 0, 3, 0, 3];
        let strings = [
            "stress_arr",
            "Stress Array",
            "sx",
            "sy",
            "sx",
            "X",
            "sy",
            "Y",
        ];
        let payload = build_dict_payload(&ints, &strings);
        let (bytes, dir) = build_dir_with(&payload);

        let table = SvarTable::build(&bytes, &dir, h()).unwrap();
        let sa = table.get("stress_arr").unwrap();
        match &sa.agg {
            SvarAgg::VecArray { dims, comps } => {
                assert_eq!(dims, &vec![3]);
                assert_eq!(comps.len(), 2);
            }
            _ => panic!("expected VecArray"),
        }
        assert_eq!(sa.atoms, 6);
    }

    #[test]
    fn rejects_short_payload() {
        // Only enough bytes for the 2-int header — int stream claims
        // 4 words total but the payload won't contain them.
        let ints = [0i32, 3]; // claim a scalar agg but no name in chars
        let mut bytes = vec![0u8; 16];
        // qty_int_words = 4 (so we claim 2 svar ints), qty_char_bytes
        // = 0 → cannot parse anything.
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&ints[0].to_le_bytes());
        bytes.extend_from_slice(&ints[1].to_le_bytes());
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![entry(16, (bytes.len() - 16) as i64)],
            names: NamePool::parse(b"", 0).unwrap(),
        };
        assert!(SvarTable::build(&bytes, &dir, h()).is_err());
    }
}
