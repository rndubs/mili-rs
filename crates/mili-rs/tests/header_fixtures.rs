//! Header parser parity against the reference corpus.
//!
//! Fixtures live in the `reference/mili-python` submodule. If the
//! submodule has not been checked out (developer cloned without
//! `--recurse-submodules`), these tests skip rather than fail so
//! `cargo test --workspace` stays green on a partial checkout. CI
//! initializes the submodule and exercises the real paths.

use std::fs;
use std::path::{Path, PathBuf};

use mili_rs::{Endianness, Header, PartitionScheme, PrecisionLimit};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data")
        .join("serial")
}

fn read_header_bytes(path: &Path) -> Option<[u8; Header::SIZE]> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < Header::SIZE {
        return None;
    }
    let mut buf = [0u8; Header::SIZE];
    buf.copy_from_slice(&bytes[..Header::SIZE]);
    Some(buf)
}

#[test]
fn basic1_header() {
    let path = corpus_root().join("basic1").join("basic1.pltA");
    let Some(bytes) = read_header_bytes(&path) else {
        eprintln!("skip: fixture missing at {}", path.display());
        return;
    };
    let h = Header::parse(&bytes).expect("basic1.pltA header parses");
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
fn dbl_nodtang_header() {
    let path = corpus_root().join("dbl_nodtang").join("dblplt000A");
    let Some(bytes) = read_header_bytes(&path) else {
        eprintln!("skip: fixture missing at {}", path.display());
        return;
    };
    let h = Header::parse(&bytes).expect("dbl_nodtang header parses");
    assert_eq!(h.header_version, 3);
    assert_eq!(h.dir_version, 3);
    assert_eq!(h.precision_limit, PrecisionLimit::Double);
    // PREC_LIMIT_DOUBLE still resolves M_FLOAT to 4 bytes; doubles are
    // opt-in per svar. See planning/shared/format.md § Numeric types.
    assert_eq!(h.float_size(), 4);
    // A-file is `dblplt000A`, state file is `dblplt00000`: the 2-digit
    // trailing state suffix matches byte 8.
    assert_eq!(h.suffix_width, 2);
}
