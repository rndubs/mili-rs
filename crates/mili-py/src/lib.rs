//! `milox._native` — the Rust extension module backing the `milox`
//! Python package. M1: `open_database()` shim support
//! (`PyMiliDatabase`) + read-only metadata accessors + the three-class
//! error hierarchy. Bulk arrays / `query()` are M2+.

// pyo3 0.22's `#[pymethods]` trampolines call `.into()` on the
// returned `PyErr`; clippy 1.83+ flags that as a useless self-
// conversion. The code lives in macro-generated wrappers outside any
// addressable item, so the allow must be crate-level.
#![allow(clippy::useless_conversion)]

mod database;
mod errors;

use pyo3::prelude::*;

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<database::PyMiliDatabase>()?;
    m.add("MiliError", m.py().get_type_bound::<errors::MiliError>())?;
    m.add(
        "MiliFileNotFoundError",
        m.py().get_type_bound::<errors::MiliFileNotFoundError>(),
    )?;
    m.add(
        "MiliAParseError",
        m.py().get_type_bound::<errors::MiliAParseError>(),
    )?;
    m.add(
        "MiliPythonError",
        m.py().get_type_bound::<errors::MiliPythonError>(),
    )?;
    Ok(())
}
