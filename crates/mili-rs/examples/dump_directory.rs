//! Diagnostic: parse a `.A` file's header + directory and dump a
//! summary to stdout. Use during incremental bring-up.
//!
//! `cargo run --example dump_directory -- path/to/run.pltA`

use std::env;
use std::fs;
use std::process::ExitCode;

use mili_rs::{Directory, Header};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: dump_directory <path/to/.A file>");
        return ExitCode::from(2);
    };
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let header = match Header::parse(&bytes) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("header parse: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("header: {header:?}");
    let dir = match Directory::parse(&bytes, &header) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("directory parse: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "directory: commit={}, qty_entries={}, qty_states={}, names={}, state_map_bytes={}",
        dir.commit_count,
        dir.entries.len(),
        dir.qty_states,
        dir.names.len(),
        dir.state_map.len(),
    );
    for (i, e) in dir.entries.iter().enumerate().take(40) {
        let names: Vec<&str> = (e.name_start..e.name_start + e.name_count)
            .map(|j| dir.names.get(j as usize))
            .collect();
        println!(
            "  [{i:3}] {:>16?} m1={} m2={} off={} len={} names={:?}",
            e.entry_type, e.modifier1, e.modifier2, e.offset, e.length, names,
        );
    }
    if dir.entries.len() > 40 {
        println!("  ... and {} more", dir.entries.len() - 40);
    }
    ExitCode::SUCCESS
}
