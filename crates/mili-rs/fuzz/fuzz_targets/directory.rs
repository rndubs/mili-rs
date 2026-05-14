#![no_main]
//! Fuzz target for `Directory::parse`.
//!
//! Header parse is part of the harness — we feed the same input
//! bytes to `Header::parse` first, then on success let the directory
//! parser see the rest. Inputs whose first 16 bytes don't form a
//! valid header short-circuit and won't reach the directory walker;
//! that matches the production flow in `Database::open` and keeps
//! the fuzzer focused on directory-payload corruption rather than
//! header-byte corruption (which `header` already covers).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(header) = mili_rs::Header::parse(data) {
        let _ = mili_rs::Directory::parse(data, &header);
    }
});
