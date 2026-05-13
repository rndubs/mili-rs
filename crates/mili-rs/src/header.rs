//! `.A` file header (16 bytes). Pure parsing, no I/O.
//!
//! See `planning/shared/format.md` § "`.A` header" for the byte layout.
//! The header's endianness byte drives every numeric read in the rest
//! of the file; `Header::float_size` / `int_size` resolve the
//! `M_FLOAT` / `M_INT` aliases for downstream parsers.

use crate::error::{MiliError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub header_version: u8,
    pub dir_version: u8,
    pub endianness: Endianness,
    pub precision_limit: PrecisionLimit,
    pub suffix_width: u8,
    pub partition_scheme: PartitionScheme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endianness {
    Big,
    Little,
}

/// Resolution of the `M_FLOAT` / `M_INT` aliases at open time.
///
/// Resolved before Step 1 against the C source and the `dbl_nodtang`
/// fixture: under `SINGLE` and `DOUBLE` both, `M_FLOAT` is 4 bytes and
/// `M_INT` is 4 bytes. Double precision is opt-in per svar via
/// `M_FLOAT8`. See `planning/shared/format.md` § "Numeric types".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionLimit {
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionScheme {
    StateCount,
    ByteCount,
}

impl Header {
    pub const SIZE: usize = 16;
    pub const MAGIC: &'static [u8; 4] = b"mili";

    /// Lowest dir version the reader accepts. v1 is deferred — see the
    /// resolved-questions section of `planning/mili-rs/plan.md`.
    pub const MIN_DIR_VERSION: u8 = 2;
    pub const MAX_DIR_VERSION: u8 = 3;
    pub const MIN_HEADER_VERSION: u8 = 2;
    pub const MAX_HEADER_VERSION: u8 = 3;

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < Self::SIZE {
            return Err(MiliError::HeaderTooShort(bytes.len()));
        }
        let magic: [u8; 4] = bytes[0..4].try_into().expect("4 bytes");
        if &magic != Self::MAGIC {
            return Err(MiliError::BadMagic(magic));
        }
        let header_version = bytes[4];
        if !(Self::MIN_HEADER_VERSION..=Self::MAX_HEADER_VERSION).contains(&header_version) {
            return Err(MiliError::UnsupportedHeader(header_version));
        }
        let dir_version = bytes[5];
        if !(Self::MIN_DIR_VERSION..=Self::MAX_DIR_VERSION).contains(&dir_version) {
            return Err(MiliError::UnsupportedDir(dir_version));
        }
        let endianness = match bytes[6] {
            1 => Endianness::Big,
            2 => Endianness::Little,
            b => return Err(MiliError::UnsupportedEndianness(b)),
        };
        let precision_limit = match bytes[7] {
            1 => PrecisionLimit::Single,
            2 => PrecisionLimit::Double,
            b => return Err(MiliError::UnsupportedPrecisionLimit(b)),
        };
        let suffix_width = bytes[8];
        if suffix_width == 0 {
            return Err(MiliError::InvalidSuffixWidth);
        }
        let partition_scheme = match bytes[9] {
            1 => PartitionScheme::StateCount,
            2 => PartitionScheme::ByteCount,
            b => return Err(MiliError::UnsupportedPartitionScheme(b)),
        };
        let ext_count = bytes[15];
        if ext_count != 0 {
            return Err(MiliError::HeaderExtensionUnsupported(ext_count));
        }
        Ok(Self {
            header_version,
            dir_version,
            endianness,
            precision_limit,
            suffix_width,
            partition_scheme,
        })
    }

    /// On-disk width of `M_FLOAT` for this database.
    pub fn float_size(&self) -> usize {
        4
    }

    /// On-disk width of `M_INT` for this database.
    pub fn int_size(&self) -> usize {
        4
    }

    pub fn is_native_endian(&self) -> bool {
        match self.endianness {
            Endianness::Big => cfg!(target_endian = "big"),
            Endianness::Little => cfg!(target_endian = "little"),
        }
    }
}

impl Endianness {
    #[inline]
    pub fn read_i32(self, bytes: &[u8; 4]) -> i32 {
        match self {
            Endianness::Big => i32::from_be_bytes(*bytes),
            Endianness::Little => i32::from_le_bytes(*bytes),
        }
    }

    #[inline]
    pub fn read_i64(self, bytes: &[u8; 8]) -> i64 {
        match self {
            Endianness::Big => i64::from_be_bytes(*bytes),
            Endianness::Little => i64::from_le_bytes(*bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_bytes() -> [u8; 16] {
        [
            b'm', b'i', b'l', b'i', // magic
            3,    // header version
            3,    // dir version
            2,    // little-endian
            2,    // PREC_LIMIT_DOUBLE
            2,    // suffix width
            1,    // STATE_COUNT
            0, 0, 0, 0, 0, // reserved
            0, // extension count
        ]
    }

    #[test]
    fn parses_canonical_header() {
        let h = Header::parse(&valid_bytes()).unwrap();
        assert_eq!(h.header_version, 3);
        assert_eq!(h.dir_version, 3);
        assert_eq!(h.endianness, Endianness::Little);
        assert_eq!(h.precision_limit, PrecisionLimit::Double);
        assert_eq!(h.suffix_width, 2);
        assert_eq!(h.partition_scheme, PartitionScheme::StateCount);
        assert_eq!(h.float_size(), 4);
        assert_eq!(h.int_size(), 4);
    }

    #[test]
    fn rejects_short_input() {
        let err = Header::parse(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, MiliError::HeaderTooShort(10)));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = valid_bytes();
        bytes[0] = b'j';
        let err = Header::parse(&bytes).unwrap_err();
        assert!(matches!(err, MiliError::BadMagic(m) if m == *b"jili"));
    }

    #[test]
    fn rejects_dir_v1() {
        let mut bytes = valid_bytes();
        bytes[5] = 1;
        assert!(matches!(
            Header::parse(&bytes).unwrap_err(),
            MiliError::UnsupportedDir(1)
        ));
    }

    #[test]
    fn rejects_prec_null_quad_none() {
        for b in [0u8, 3, 4, 5] {
            let mut bytes = valid_bytes();
            bytes[7] = b;
            assert!(matches!(
                Header::parse(&bytes).unwrap_err(),
                MiliError::UnsupportedPrecisionLimit(x) if x == b
            ));
        }
    }

    #[test]
    fn rejects_zero_suffix_width() {
        let mut bytes = valid_bytes();
        bytes[8] = 0;
        assert!(matches!(
            Header::parse(&bytes).unwrap_err(),
            MiliError::InvalidSuffixWidth
        ));
    }

    #[test]
    fn rejects_unknown_endianness() {
        let mut bytes = valid_bytes();
        bytes[6] = 7;
        assert!(matches!(
            Header::parse(&bytes).unwrap_err(),
            MiliError::UnsupportedEndianness(7)
        ));
    }

    #[test]
    fn rejects_unknown_partition_scheme() {
        let mut bytes = valid_bytes();
        bytes[9] = 9;
        assert!(matches!(
            Header::parse(&bytes).unwrap_err(),
            MiliError::UnsupportedPartitionScheme(9)
        ));
    }

    #[test]
    fn rejects_extension_fields() {
        let mut bytes = valid_bytes();
        bytes[15] = 1;
        assert!(matches!(
            Header::parse(&bytes).unwrap_err(),
            MiliError::HeaderExtensionUnsupported(1)
        ));
    }

    #[test]
    fn accepts_big_endian() {
        let mut bytes = valid_bytes();
        bytes[6] = 1;
        let h = Header::parse(&bytes).unwrap();
        assert_eq!(h.endianness, Endianness::Big);
    }

    #[test]
    fn accepts_single_precision() {
        let mut bytes = valid_bytes();
        bytes[7] = 1;
        let h = Header::parse(&bytes).unwrap();
        assert_eq!(h.precision_limit, PrecisionLimit::Single);
        assert_eq!(h.float_size(), 4);
    }
}
