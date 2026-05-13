use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum MiliError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("bad magic: expected b\"mili\", got {0:?}")]
    BadMagic([u8; 4]),

    #[error("unsupported header version {0}")]
    UnsupportedHeader(u8),

    #[error("unsupported directory version {0}")]
    UnsupportedDir(u8),

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
