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
