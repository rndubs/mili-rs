#![no_main]
//! Fuzz target for `Header::parse`.
//!
//! Goal: every byte sequence either parses into a `Header` or
//! returns a typed `MiliError`. No panics, no OOMs, no hangs.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = mili_rs::Header::parse(data);
});
