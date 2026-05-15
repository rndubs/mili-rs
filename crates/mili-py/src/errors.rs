//! `MiliError` → the three-class Python exception hierarchy.
//!
//! Mirrors upstream `mili` (`afileIO.py:27-30`,
//! `milidatabase.py:36-38`): three exception classes. We add a common
//! `MiliError` base for convenience `except` clauses — the concrete
//! classes are still `Exception` subclasses, so `except
//! mili.MiliFileNotFoundError` style code ports unchanged.
//!
//! Typed `MiliError` detail (incl. the Phase-1.5 `NoFragments` /
//! `FragmentMismatch` and the Phase-2-inherited `InconsistentIpCounts`)
//! is preserved verbatim in the message via the variant's `Display`.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::PyErr;

use mili_rs::MiliError as CoreError;

create_exception!(_native, MiliError, PyException);
create_exception!(_native, MiliFileNotFoundError, MiliError);
create_exception!(_native, MiliAParseError, MiliError);
create_exception!(_native, MiliPythonError, MiliError);

/// Convert a `mili-rs` error into the matching Python exception,
/// preserving the typed message.
pub fn to_pyerr(err: &CoreError) -> PyErr {
    let msg = err.to_string();
    match err {
        // Upstream raises `MiliFileNotFoundError` both for a missing
        // path and for "no A-files matched" (`reader.py:50,55`); map
        // `NoFragments` there for drop-in parity.
        CoreError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            MiliFileNotFoundError::new_err(msg)
        }
        CoreError::NoFragments { .. } => MiliFileNotFoundError::new_err(msg),

        CoreError::BadMagic(_)
        | CoreError::HeaderTooShort(_)
        | CoreError::UnsupportedHeader(_)
        | CoreError::UnsupportedDir(_)
        | CoreError::UnsupportedEndianness(_)
        | CoreError::UnsupportedPrecisionLimit(_)
        | CoreError::InvalidSuffixWidth
        | CoreError::UnsupportedPartitionScheme(_)
        | CoreError::HeaderExtensionUnsupported(_)
        | CoreError::MalformedDirectory(_)
        | CoreError::UnknownEntryType(_)
        | CoreError::Truncated { .. }
        | CoreError::DirEntryOutOfRange { .. }
        | CoreError::BadName(_) => MiliAParseError::new_err(msg),

        // Everything else — query/validation errors plus the typed
        // Phase-2 detail variants — maps to `MiliPythonError`.
        _ => MiliPythonError::new_err(msg),
    }
}
