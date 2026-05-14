use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum MiliError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("bad magic: expected b\"mili\", got {0:?}")]
    BadMagic([u8; 4]),

    #[error("header too short: need {need} bytes, got {got}", need = crate::header::Header::SIZE, got = .0)]
    HeaderTooShort(usize),

    #[error("unsupported header version {0}")]
    UnsupportedHeader(u8),

    #[error("unsupported directory version {0}")]
    UnsupportedDir(u8),

    #[error("unsupported endianness byte {0:#x}")]
    UnsupportedEndianness(u8),

    #[error("unsupported precision limit byte {0:#x}")]
    UnsupportedPrecisionLimit(u8),

    #[error("invalid state-file suffix width: 0")]
    InvalidSuffixWidth,

    #[error("unsupported partition scheme byte {0:#x}")]
    UnsupportedPartitionScheme(u8),

    #[error("header extension fields not supported (count={0})")]
    HeaderExtensionUnsupported(u8),

    #[error("malformed directory: {0}")]
    MalformedDirectory(&'static str),

    #[error("unknown directory entry type code {0}")]
    UnknownEntryType(i64),

    #[error("truncated {file}: needed {need} bytes at offset {off}, got {got}")]
    Truncated {
        file: PathBuf,
        off: u64,
        need: usize,
        got: usize,
    },

    #[error("directory entry {idx} points past EOF (offset {off}, len {len}, file size {size})")]
    DirEntryOutOfRange {
        idx: usize,
        off: u64,
        len: u64,
        size: u64,
    },

    #[error("bad UTF-8 in name pool at offset {0}")]
    BadName(usize),

    #[error("unknown svar {0:?}")]
    UnknownSvar(String),

    #[error("unknown class {0:?}")]
    UnknownClass(String),

    #[error("state {0} out of range (0..{1})")]
    StateOutOfRange(usize, usize),

    #[error("misaligned mmap: offset {0} not aligned for {1}-byte type")]
    Misaligned(usize, usize),

    #[error("query feature not implemented yet: {0}")]
    Unsupported(&'static str),

    #[error("no subrecord covers svar {svar:?} on class {class:?}")]
    NoMatchingSubrec { svar: String, class: String },

    #[error("label {label} not found on class {class:?}")]
    LabelNotFound { label: i32, class: String },

    #[error("integration-point index {ip} out of range (svar has {atoms} per-IP slots)")]
    IpOutOfRange { ip: usize, atoms: usize },

    #[error("ips filter is only valid against vec_array svars; svar {svar:?} is {agg}")]
    IpFilterNotApplicable { svar: String, agg: &'static str },

    #[error("material {material} not declared")]
    UnknownMaterial { material: i32 },

    #[error("invalid svar subscript {input:?}: {reason}")]
    InvalidSubscript { input: String, reason: &'static str },

    #[error("subscript notation not applicable to svar {svar:?} ({agg} agg)")]
    SubscriptNotApplicable { svar: String, agg: &'static str },

    /// Different subrecords carrying the requested svar on the same
    /// class report different per-object atom widths (i.e. inconsistent
    /// integration-point counts across materials). Without an explicit
    /// `ips` filter the result shape is ambiguous, so the query is
    /// rejected. Mirrors mili-python's `ValueError` for
    /// `query("sx", "brick")` on `basic1` where material 5 has 8 IPs
    /// and material 7 has 9 (`test_bugfixes.py:99-117`).
    #[error(
        "svar {svar:?} on class {class:?} has inconsistent integration-point counts across \
         subrecords ({counts:?}); pass an explicit `ips` filter to disambiguate"
    )]
    InconsistentIpCounts {
        svar: String,
        class: String,
        counts: Vec<usize>,
    },
}

pub type Result<T, E = MiliError> = core::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_bad_magic() {
        let err = MiliError::BadMagic(*b"junk");
        assert_eq!(
            err.to_string(),
            "bad magic: expected b\"mili\", got [106, 117, 110, 107]"
        );
    }

    #[test]
    fn io_error_conversion() {
        let io = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short");
        let err: MiliError = io.into();
        assert!(matches!(err, MiliError::Io(_)));
    }
}
