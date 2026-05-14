#![no_main]
//! Fuzz target for `ParamValue::decode`.
//!
//! Builds a directory off the same input bytes (so each fuzz step
//! also re-fuzzes header + directory parsing) then iterates every
//! parsed `DirEntry` and runs `ParamValue::decode` against it.
//! Non-param entry types return typed errors; the goal is that
//! malicious offset / length / agg / dtype combinations never
//! trigger a panic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(header) = mili_rs::Header::parse(data) else {
        return;
    };
    let Ok(dir) = mili_rs::Directory::parse(data, &header) else {
        return;
    };
    for entry in &dir.entries {
        let _ = mili_rs::ParamValue::decode(data, entry, header);
    }
});
