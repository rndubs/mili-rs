//! Trailer-based directory parser for the `.A` (and `.ATI*`) files.
//!
//! See `planning/shared/format.md` § "Directory" and § "Entry types in
//! the directory" for the byte layout. The trailer lives at EOF; we
//! walk backward to recover the directory header, the entries, and
//! the name pool.

use crate::error::{MiliError, Result};
use crate::header::Header;

/// Trailer layout, v2+: four 4-byte ints (NAMES_LEN, COMMIT_COUNT,
/// QTY_ENTRIES, QTY_STATES). v1 omits QTY_STATES and is rejected at
/// the header stage.
const TRAILER_FIELDS: usize = 4;
const TRAILER_BYTES: usize = TRAILER_FIELDS * 4;
const ENTRY_FIELDS: usize = 6;

/// State_descriptor on disk: `int file (4) + LONGLONG offset (8) +
/// float time (4) + int srec_format (4)` = 20 bytes
/// (`reference/mili/src/mili_internal.h:109-115`).
const STATE_MAP_ENTRY_BYTES: usize = 20;

#[derive(Debug)]
pub struct Directory {
    pub commit_count: i32,
    /// Raw `QTY_STATES` from the trailer. For header v3+ databases
    /// that ship a `R.A.tfile`, the real state count comes from that
    /// file and this value is 0 by writer convention — reconciliation
    /// is the family-open layer's job, not this parser's.
    pub qty_states: i32,
    /// Byte range of the state-map block embedded between entries
    /// and trailer. Empty when `qty_states` is 0 (either no states
    /// or v3-with-tfile).
    pub state_map: ByteRange,
    pub entries: Vec<DirEntry>,
    pub names: NamePool,
}

/// Byte range `[start, end)` within the parent file.
#[derive(Debug, Clone, Copy)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl ByteRange {
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i64)]
pub enum DirEntryType {
    Nodes = 0,
    ElemConns = 1,
    ClassIdents = 2,
    StateVarDict = 3,
    StateRecData = 4,
    MiliParam = 5,
    ApplicationParam = 6,
    ClassDef = 7,
    SurfaceConns = 8,
    TiParam = 9,
}

impl DirEntryType {
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Nodes,
            1 => Self::ElemConns,
            2 => Self::ClassIdents,
            3 => Self::StateVarDict,
            4 => Self::StateRecData,
            5 => Self::MiliParam,
            6 => Self::ApplicationParam,
            7 => Self::ClassDef,
            8 => Self::SurfaceConns,
            9 => Self::TiParam,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub entry_type: DirEntryType,
    pub modifier1: i64,
    pub modifier2: i64,
    pub string_qty: i64,
    pub offset: i64,
    pub length: i64,
    /// Index of this entry's first name in the [`NamePool`].
    pub name_start: u32,
    /// Number of names this entry consumes from the pool.
    pub name_count: u32,
}

/// Names stored in the directory's name pool. Validated UTF-8.
#[derive(Debug)]
pub struct NamePool {
    raw: Box<[u8]>,
    spans: Box<[(u32, u32)]>,
}

impl NamePool {
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Get the `i`-th name. Panics if `i >= len()`.
    pub fn get(&self, i: usize) -> &str {
        let (s, e) = self.spans[i];
        let bytes = &self.raw[s as usize..e as usize];
        // UTF-8 validated once at parse time.
        std::str::from_utf8(bytes).expect("name-pool UTF-8 was validated at parse time")
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).map(|i| self.get(i))
    }
}

impl Directory {
    /// Parse the directory out of a `.A` file already loaded into
    /// memory. `bytes` is the entire file contents; `header` was
    /// parsed off the start of those same bytes.
    pub fn parse(bytes: &[u8], header: &Header) -> Result<Self> {
        let entry_word_size = match header.dir_version {
            2 => 4usize,
            3 => 8usize,
            v => return Err(MiliError::UnsupportedDir(v)),
        };

        let file_size = bytes.len();
        if file_size < Header::SIZE + TRAILER_BYTES {
            return Err(MiliError::MalformedDirectory(
                "file shorter than header + trailer",
            ));
        }

        // ---- Trailer ----------------------------------------------------
        let trailer_start = file_size - TRAILER_BYTES;
        let end = header.endianness;
        let names_len = end.read_i32(slice4(bytes, trailer_start)?);
        let commit_count = end.read_i32(slice4(bytes, trailer_start + 4)?);
        let qty_entries = end.read_i32(slice4(bytes, trailer_start + 8)?);
        let qty_states = end.read_i32(slice4(bytes, trailer_start + 12)?);

        if qty_entries < 1 {
            return Err(MiliError::MalformedDirectory("qty_entries < 1"));
        }
        if names_len < 0 {
            return Err(MiliError::MalformedDirectory("names_len < 0"));
        }
        let qty_entries = qty_entries as usize;
        let names_len = names_len as usize;
        let entries_bytes = qty_entries
            .checked_mul(ENTRY_FIELDS)
            .and_then(|n| n.checked_mul(entry_word_size))
            .ok_or(MiliError::MalformedDirectory("entry length overflow"))?;

        // The state-map block lives between entries and trailer when
        // `qty_states > 0`. v3 databases that ship a `.tfile` write
        // `qty_states = 0` here (`direc.c:218-222`), so this branch
        // collapses to zero size and we can derive layout from the
        // trailer field alone. See `direc.c:487-503`.
        let states_size = if qty_states > 0 {
            (qty_states as usize)
                .checked_mul(STATE_MAP_ENTRY_BYTES)
                .ok_or(MiliError::MalformedDirectory("state map size overflow"))?
        } else {
            0
        };

        let state_map_end = trailer_start;
        let state_map_start = state_map_end
            .checked_sub(states_size)
            .ok_or(MiliError::MalformedDirectory("state map before file start"))?;
        let entries_start = state_map_start
            .checked_sub(entries_bytes)
            .ok_or(MiliError::MalformedDirectory("entries before file start"))?;
        let names_start = entries_start
            .checked_sub(names_len)
            .ok_or(MiliError::MalformedDirectory("name pool before file start"))?;
        if names_start < Header::SIZE {
            return Err(MiliError::MalformedDirectory(
                "directory overlaps fixed header",
            ));
        }

        let (entries, total_names) =
            parse_entries(bytes, entries_start, qty_entries, entry_word_size, end)?;

        let pool_bytes = &bytes[names_start..entries_start];
        let names = NamePool::parse(pool_bytes, total_names)?;

        Ok(Self {
            commit_count,
            qty_states,
            state_map: ByteRange {
                start: state_map_start,
                end: state_map_end,
            },
            entries,
            names,
        })
    }
}

impl NamePool {
    fn parse(bytes: &[u8], expected_count: u32) -> Result<Self> {
        if std::str::from_utf8(bytes).is_err() {
            // We don't yet know the offset of the first invalid byte; the
            // common case (NUL terminators present, ASCII names) is fine,
            // so this is a coarse error for now.
            return Err(MiliError::BadName(0));
        }
        let mut spans: Vec<(u32, u32)> = Vec::with_capacity(expected_count as usize);
        let mut start: usize = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if b == 0 {
                if spans.len() < expected_count as usize {
                    spans.push((start as u32, i as u32));
                }
                start = i + 1;
            }
        }
        if (spans.len() as u32) < expected_count {
            return Err(MiliError::MalformedDirectory(
                "name pool ended before all entries' names were read",
            ));
        }
        Ok(Self {
            raw: bytes.to_vec().into_boxed_slice(),
            spans: spans.into_boxed_slice(),
        })
    }
}

fn parse_entries(
    bytes: &[u8],
    entries_start: usize,
    qty_entries: usize,
    word: usize,
    end: crate::header::Endianness,
) -> Result<(Vec<DirEntry>, u32)> {
    let read = |off: usize| -> Result<i64> {
        match word {
            4 => Ok(i64::from(end.read_i32(slice4(bytes, off)?))),
            8 => Ok(end.read_i64(slice8(bytes, off)?)),
            _ => unreachable!(),
        }
    };
    let mut entries = Vec::with_capacity(qty_entries);
    let mut name_cursor: u32 = 0;
    for i in 0..qty_entries {
        let base = entries_start + i * ENTRY_FIELDS * word;
        let type_code = read(base)?;
        let entry_type =
            DirEntryType::from_code(type_code).ok_or(MiliError::UnknownEntryType(type_code))?;
        let modifier1 = read(base + word)?;
        let modifier2 = read(base + 2 * word)?;
        let string_qty = read(base + 3 * word)?;
        let offset = read(base + 4 * word)?;
        let length = read(base + 5 * word)?;
        if string_qty < 0 {
            return Err(MiliError::MalformedDirectory("entry string_qty < 0"));
        }
        let name_count = u32::try_from(string_qty)
            .map_err(|_| MiliError::MalformedDirectory("entry string_qty too large"))?;
        let name_start = name_cursor;
        name_cursor = name_cursor
            .checked_add(name_count)
            .ok_or(MiliError::MalformedDirectory(
                "total name count overflowed u32",
            ))?;
        entries.push(DirEntry {
            entry_type,
            modifier1,
            modifier2,
            string_qty,
            offset,
            length,
            name_start,
            name_count,
        });
    }
    Ok((entries, name_cursor))
}

fn slice4(bytes: &[u8], off: usize) -> Result<&[u8; 4]> {
    bytes
        .get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(MiliError::MalformedDirectory("read past EOF"))
}

fn slice8(bytes: &[u8], off: usize) -> Result<&[u8; 8]> {
    bytes
        .get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .ok_or(MiliError::MalformedDirectory("read past EOF"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Endianness, PartitionScheme, PrecisionLimit};

    fn header_v(dir_version: u8) -> Header {
        Header {
            header_version: 3,
            dir_version,
            endianness: Endianness::Little,
            precision_limit: PrecisionLimit::Double,
            suffix_width: 2,
            partition_scheme: PartitionScheme::StateCount,
        }
    }

    /// Build a synthetic .A file:
    /// 16-byte stub header padding + names + entries + trailer.
    fn build_dir(
        dir_version: u8,
        names: &[&str],
        entries: &[(DirEntryType, i64, i64, i64, i64, i64)],
    ) -> Vec<u8> {
        let entry_word = if dir_version == 3 { 8 } else { 4 };
        let mut buf = vec![0u8; Header::SIZE];

        // name pool: NUL-terminated concatenation
        for n in names {
            buf.extend_from_slice(n.as_bytes());
            buf.push(0);
        }
        let names_len = (names.iter().map(|s| s.len() + 1).sum::<usize>()) as i32;

        // entries
        let mut string_qty_running: i64 = 0;
        for (etype, modifier1, modifier2, string_qty, offset, length) in entries.iter().copied() {
            let words: [i64; 6] = [
                etype as i64,
                modifier1,
                modifier2,
                string_qty,
                offset,
                length,
            ];
            for w in words {
                if entry_word == 8 {
                    buf.extend_from_slice(&w.to_le_bytes());
                } else {
                    buf.extend_from_slice(&(w as i32).to_le_bytes());
                }
            }
            string_qty_running += string_qty;
        }
        assert_eq!(
            string_qty_running as usize,
            names.len(),
            "test setup: name count mismatches entries"
        );

        // trailer — qty_states = 0 so we don't have to include the
        // 20-byte-per-state map block in the synthetic file.
        buf.extend_from_slice(&names_len.to_le_bytes());
        buf.extend_from_slice(&7i32.to_le_bytes()); // commit_count
        buf.extend_from_slice(&(entries.len() as i32).to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes()); // qty_states

        buf
    }

    #[test]
    fn v3_single_entry_round_trip() {
        let bytes = build_dir(
            3,
            &["mesh dimensions"],
            &[(DirEntryType::MiliParam, 1, 0, 1, 100, 4)],
        );
        let dir = Directory::parse(&bytes, &header_v(3)).unwrap();
        assert_eq!(dir.commit_count, 7);
        assert_eq!(dir.qty_states, 0);
        assert!(dir.state_map.is_empty());
        assert_eq!(dir.entries.len(), 1);
        let e = &dir.entries[0];
        assert_eq!(e.entry_type, DirEntryType::MiliParam);
        assert_eq!(e.modifier1, 1);
        assert_eq!(e.offset, 100);
        assert_eq!(e.length, 4);
        assert_eq!(dir.names.len(), 1);
        assert_eq!(dir.names.get(0), "mesh dimensions");
    }

    #[test]
    fn v2_widens_to_i64() {
        let bytes = build_dir(
            2,
            &["a", "b"],
            &[
                (DirEntryType::Nodes, 0, 5, 1, 16, 100),
                (DirEntryType::ClassDef, 1, 1, 1, 116, 0),
            ],
        );
        let dir = Directory::parse(&bytes, &header_v(2)).unwrap();
        assert_eq!(dir.entries.len(), 2);
        assert_eq!(dir.entries[0].length, 100);
        assert_eq!(dir.entries[1].entry_type, DirEntryType::ClassDef);
        assert_eq!(dir.names.iter().collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn rejects_zero_entries() {
        // Hand-build a degenerate trailer with qty_entries = 0.
        let mut buf = vec![0u8; Header::SIZE];
        buf.extend_from_slice(&0i32.to_le_bytes()); // names_len
        buf.extend_from_slice(&1i32.to_le_bytes()); // commit
        buf.extend_from_slice(&0i32.to_le_bytes()); // qty_entries
        buf.extend_from_slice(&0i32.to_le_bytes()); // qty_states
        let err = Directory::parse(&buf, &header_v(3)).unwrap_err();
        assert!(matches!(err, MiliError::MalformedDirectory(_)));
    }

    #[test]
    fn rejects_unknown_entry_type() {
        let mut bytes = build_dir(3, &["x"], &[(DirEntryType::Nodes, 0, 0, 1, 0, 0)]);
        // overwrite the TYPE field of entry 0 with a bogus code (99)
        let entries_offset = Header::SIZE + "x\0".len();
        let bogus = 99i64.to_le_bytes();
        bytes[entries_offset..entries_offset + 8].copy_from_slice(&bogus);
        let err = Directory::parse(&bytes, &header_v(3)).unwrap_err();
        assert!(matches!(err, MiliError::UnknownEntryType(99)));
    }

    #[test]
    fn rejects_truncated_file() {
        let bytes = vec![0u8; 4];
        let err = Directory::parse(&bytes, &header_v(3)).unwrap_err();
        assert!(matches!(err, MiliError::MalformedDirectory(_)));
    }

    #[test]
    fn parses_inline_state_map_block() {
        // Mimic v3-without-tfile or v2: qty_states > 0 in the trailer
        // means a 20-byte-per-state block sits between entries and
        // trailer.
        let entry_word = 8usize;
        let qty_states = 3i32;
        let names = &["mesh dimensions"];
        let mut buf = vec![0u8; Header::SIZE];
        // names
        buf.extend_from_slice(b"mesh dimensions\0");
        // one entry
        let words: [i64; 6] = [DirEntryType::MiliParam as i64, 1, 0, 1, 100, 4];
        for w in words {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        // state-map block
        let state_map_offset = buf.len();
        buf.extend_from_slice(&[0u8; 20 * 3]);
        // trailer
        buf.extend_from_slice(
            &(names.iter().map(|s| s.len() + 1).sum::<usize>() as i32).to_le_bytes(),
        );
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.extend_from_slice(&qty_states.to_le_bytes());

        let dir = Directory::parse(&buf, &header_v(3)).unwrap();
        assert_eq!(dir.qty_states, qty_states);
        assert_eq!(dir.state_map.start, state_map_offset);
        assert_eq!(dir.state_map.len(), 60);
        assert_eq!(dir.entries.len(), 1);
        assert_eq!(dir.names.get(0), "mesh dimensions");
        // Just to keep entry_word "used":
        let _ = entry_word;
    }

    #[test]
    fn name_pool_supports_empty_name() {
        let bytes = build_dir(3, &[""], &[(DirEntryType::MiliParam, 0, 0, 1, 0, 0)]);
        let dir = Directory::parse(&bytes, &header_v(3)).unwrap();
        assert_eq!(dir.names.get(0), "");
    }
}
