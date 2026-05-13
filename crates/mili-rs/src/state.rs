//! State-map parsing: the per-state index telling us which state file,
//! which byte offset, what timestamp, and which state-record format each
//! state was written into.
//!
//! Two storage layouts exist in v3 databases — picking the right one is
//! the directory parser's job (it exposes both `state_map` and
//! `qty_states`); this module only knows how to read the bytes once
//! they're located.
//!
//! - **Inline** (v2 always; v3 when no `.tfile` is written): the
//!   `qty_states * 20` byte block sits in the main `.A` file between
//!   the directory entries and the trailer. The 4-int trailer's
//!   `QTY_STATES` field is non-zero and tells us how many entries are
//!   present (`reference/mili/src/mili_statemap.c:633-649`). No
//!   end-of-file marker.
//!
//! - **External `.tfile`** (`<root>T`): for header v3+ databases
//!   running with `write_tfile = TRUE`, the state map gets written to
//!   a sibling file ending in `T`. The file contains
//!   `qty_states * 20` bytes followed by a single `~` (0x7E) byte
//!   (`reference/mili/src/mili.c:247-255, 276` for the name and
//!   marker; `mili_statemap.c:586-624` for the read path). The main
//!   `.A` directory's `QTY_STATES` reads as 0 in this case.
//!
//! Per-state record on disk is laid out as
//! (`reference/mili/src/mili_internal.h:109-115`,
//! `reference/mili/src/mili_statemap.c:653-664`):
//!
//! ```text
//! [ i32 file       ]   index of the state file (R.A00, R.A01, …)
//! [ i64 offset     ]   byte offset of the state header within that file
//! [ f32 time       ]   simulation time at this state
//! [ i32 srec_format]   state-record format id this state was written into
//! ```
//!
//! Note the `i64` for offset and the `f32` for time — these widths are
//! fixed regardless of the header's M_INT / M_FLOAT alias resolution
//! (the C reader uses `M_INT8` and `M_FLOAT` from the precision-resolved
//! table, but per the resolved question both resolve to the literal
//! widths we use here).

use crate::directory::{ByteRange, Directory};
use crate::error::{MiliError, Result};
use crate::header::{Endianness, Header};

/// The `~` byte that marks the end of a `<root>T` state-map file.
pub const TFILE_END_MARKER: u8 = b'~';

/// Bytes per `State_descriptor` on disk.
pub const STATE_DESCRIPTOR_BYTES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateMeta {
    /// Index of the state file holding this state (0 → `R.A00`, …).
    pub file: i32,
    /// Byte offset of the state header within that file.
    pub offset: i64,
    /// Simulation time at this state, as stored on disk.
    pub time: f32,
    /// State-record format id this state was written into.
    pub srec_format: i32,
}

impl StateMeta {
    fn read(slot: &[u8; 20], end: Endianness) -> Self {
        let file = end.read_i32(slot[0..4].try_into().expect("4 bytes"));
        let offset = end.read_i64(slot[4..12].try_into().expect("8 bytes"));
        let time = match end {
            Endianness::Big => f32::from_be_bytes(slot[12..16].try_into().expect("4 bytes")),
            Endianness::Little => f32::from_le_bytes(slot[12..16].try_into().expect("4 bytes")),
        };
        let srec_format = end.read_i32(slot[16..20].try_into().expect("4 bytes"));
        Self {
            file,
            offset,
            time,
            srec_format,
        }
    }
}

/// Where does this database's state map live?
///
/// The directory parser already tells us — it exposes `qty_states` and
/// the `state_map` byte range. If `qty_states > 0`, the bytes at
/// `state_map` are the inline state map. Otherwise the family layer
/// must look for a `<root>T` companion file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateMapSource {
    /// The state map is inline in the main `.A` file, at this byte
    /// range. The count of states is derivable from the range length.
    InlineA(ByteRange),
    /// Look at the `<root>T` companion file. The main `.A` directory's
    /// trailer carries `QTY_STATES = 0` in this mode.
    ExternalTfile,
}

impl StateMapSource {
    /// Pick the source based on the parsed directory and header.
    ///
    /// Mirrors the dispatch at `mili_statemap.c:565-583` and
    /// `srec.c:3473-3476` (only v2+ takes this path; v1 builds the
    /// state map by walking the state files — deferred along with
    /// v1 support).
    pub fn pick(header: &Header, dir: &Directory) -> Self {
        if dir.qty_states > 0 {
            Self::InlineA(dir.state_map)
        } else if header.header_version >= 3 {
            Self::ExternalTfile
        } else {
            // v2 with qty_states == 0 → an empty inline map. The byte
            // range is empty by construction in directory.rs.
            Self::InlineA(dir.state_map)
        }
    }
}

/// Parse the inline state map out of the main `.A` file's bytes.
///
/// `range` is the byte range exposed on [`Directory::state_map`]. The
/// caller computes the count from the range length — invalid lengths
/// (non-multiple of 20) are rejected here.
pub fn parse_inline(bytes: &[u8], range: ByteRange, header: &Header) -> Result<Vec<StateMeta>> {
    let slice = bytes
        .get(range.start..range.end)
        .ok_or(MiliError::Truncated {
            file: std::path::PathBuf::new(),
            off: range.start as u64,
            need: range.len(),
            got: bytes.len().saturating_sub(range.start),
        })?;
    if !slice.len().is_multiple_of(STATE_DESCRIPTOR_BYTES) {
        return Err(MiliError::MalformedDirectory(
            "state map length not a multiple of 20",
        ));
    }
    Ok(parse_run(slice, header.endianness))
}

/// Parse a `<root>T` tfile's full byte contents into a state-map
/// vector, validating the trailing `~` end-of-file marker.
///
/// The tfile layout is `state_qty * 20 bytes || one '~' byte`
/// (`reference/mili/src/mili_statemap.c:602-624`). A zero-state tfile
/// is `[0x7E]` and is also accepted (matches the C writer's
/// "create then mark" pattern at `mili.c:1100-1106`).
pub fn parse_tfile(bytes: &[u8], header: &Header) -> Result<Vec<StateMeta>> {
    let n = bytes.len();
    if n == 0 {
        return Err(MiliError::MalformedDirectory("tfile is empty"));
    }
    let body_len = n - 1;
    if !body_len.is_multiple_of(STATE_DESCRIPTOR_BYTES) {
        return Err(MiliError::MalformedDirectory(
            "tfile body length not a multiple of 20",
        ));
    }
    if bytes[body_len] != TFILE_END_MARKER {
        return Err(MiliError::MalformedDirectory(
            "tfile missing trailing '~' end-marker",
        ));
    }
    Ok(parse_run(&bytes[..body_len], header.endianness))
}

fn parse_run(slice: &[u8], end: Endianness) -> Vec<StateMeta> {
    let qty = slice.len() / STATE_DESCRIPTOR_BYTES;
    let mut out = Vec::with_capacity(qty);
    for i in 0..qty {
        let off = i * STATE_DESCRIPTOR_BYTES;
        let slot: &[u8; 20] = slice[off..off + STATE_DESCRIPTOR_BYTES]
            .try_into()
            .expect("20 bytes");
        out.push(StateMeta::read(slot, end));
    }
    out
}

/// Construct the `<root>T` path from the path to the main `.A` file.
///
/// Mirrors `reference/mili/src/mili.c:247-255` — the tfile name is the
/// family root with a literal `T` suffix. For a database whose root
/// is `basic1.plt` and whose `.A` file is `basic1.pltA`, the tfile
/// is `basic1.pltT`.
pub fn tfile_path(a_file: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = a_file.file_name()?.to_str()?;
    // The `.A` filename ends in a single `A` character — strip it and
    // append `T`.
    let stem = name.strip_suffix('A')?;
    let mut t = stem.to_owned();
    t.push('T');
    Some(a_file.with_file_name(t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Endianness, PartitionScheme, PrecisionLimit};

    fn h(ver: u8) -> Header {
        Header {
            header_version: ver,
            dir_version: 3,
            endianness: Endianness::Little,
            precision_limit: PrecisionLimit::Double,
            suffix_width: 2,
            partition_scheme: PartitionScheme::StateCount,
        }
    }

    fn descriptor_bytes(meta: StateMeta) -> [u8; 20] {
        let mut b = [0u8; 20];
        b[0..4].copy_from_slice(&meta.file.to_le_bytes());
        b[4..12].copy_from_slice(&meta.offset.to_le_bytes());
        b[12..16].copy_from_slice(&meta.time.to_le_bytes());
        b[16..20].copy_from_slice(&meta.srec_format.to_le_bytes());
        b
    }

    #[test]
    fn round_trips_one_state_inline() {
        let meta = StateMeta {
            file: 0,
            offset: 0,
            time: 1.25,
            srec_format: 0,
        };
        let mut bytes = vec![0u8; 64];
        bytes[16..36].copy_from_slice(&descriptor_bytes(meta));
        let range = ByteRange { start: 16, end: 36 };
        let metas = parse_inline(&bytes, range, &h(3)).unwrap();
        assert_eq!(metas, vec![meta]);
    }

    #[test]
    fn inline_rejects_misaligned_length() {
        let bytes = vec![0u8; 64];
        let range = ByteRange { start: 16, end: 35 };
        assert!(parse_inline(&bytes, range, &h(3)).is_err());
    }

    #[test]
    fn tfile_round_trip_with_marker() {
        let meta = StateMeta {
            file: 0,
            offset: 16,
            time: 2.0,
            srec_format: 1,
        };
        let mut tfile = Vec::new();
        tfile.extend_from_slice(&descriptor_bytes(meta));
        tfile.push(TFILE_END_MARKER);
        let metas = parse_tfile(&tfile, &h(3)).unwrap();
        assert_eq!(metas, vec![meta]);
    }

    #[test]
    fn tfile_zero_state_is_just_marker() {
        let tfile = vec![TFILE_END_MARKER];
        let metas = parse_tfile(&tfile, &h(3)).unwrap();
        assert!(metas.is_empty());
    }

    #[test]
    fn tfile_missing_marker_errors() {
        let mut tfile = vec![0u8; 20];
        tfile[19] = 0; // not '~'
        assert!(parse_tfile(&tfile, &h(3)).is_err());
    }

    #[test]
    fn tfile_misaligned_body_errors() {
        let tfile = vec![TFILE_END_MARKER; 22];
        // body length is 21, not a multiple of 20.
        assert!(parse_tfile(&tfile, &h(3)).is_err());
    }

    #[test]
    fn source_picks_inline_when_qty_states_positive() {
        let dir = Directory {
            commit_count: 1,
            qty_states: 5,
            state_map: ByteRange {
                start: 100,
                end: 200,
            },
            entries: Vec::new(),
            names: crate::directory::NamePool::parse(b"", 0).unwrap(),
        };
        match StateMapSource::pick(&h(3), &dir) {
            StateMapSource::InlineA(r) => assert_eq!(r.len(), 100),
            StateMapSource::ExternalTfile => panic!("expected inline"),
        }
    }

    #[test]
    fn source_picks_tfile_when_v3_qty_states_zero() {
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: Vec::new(),
            names: crate::directory::NamePool::parse(b"", 0).unwrap(),
        };
        assert_eq!(
            StateMapSource::pick(&h(3), &dir),
            StateMapSource::ExternalTfile,
        );
    }

    #[test]
    fn tfile_path_builds_expected_name() {
        let p = std::path::Path::new("/data/run/basic1.pltA");
        let t = tfile_path(p).unwrap();
        assert_eq!(t, std::path::Path::new("/data/run/basic1.pltT"));
    }
}
