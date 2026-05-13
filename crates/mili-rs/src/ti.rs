//! Time-independent (TI) parameter file discovery and load.
//!
//! ## Where do TI params actually live?
//!
//! The original `format.md` claimed TI params lived exclusively in
//! separate `R.ATI*` files. Inspecting the C source clarifies the
//! rule:
//!
//! - **Directory v2 and v3** (every fixture in the corpus): TI params
//!   are written as `TI_PARAM`-typed entries *inline* in the main
//!   `.A` directory. They share the same param hash table as
//!   `MILI_PARAM` / `APPLICATION_PARAM` entries
//!   (`reference/mili/src/direc.c:653-689`). The TI API
//!   (`mc_ti_read_scalar` etc.) short-circuits to the regular
//!   parameter reader whenever `DIR_VERSION_IDX > 1`
//!   (`reference/mili/src/ti.c:179-212, 298-341`).
//!
//! - **Directory v1**: TI params live in separate files. The filename
//!   pattern is `<root>_TI_<base26>` — `<root>_TI_A`, `<root>_TI_B`,
//!   …, not `R.ATI*` as the planning doc originally claimed
//!   (`reference/mili/src/mili_util.c:908-911`). The trailer layout
//!   matches the main `.A` directory but omits `QTY_STATES`
//!   (`reference/mili/src/tidirc.c:398-407`).
//!
//! Since v1 directory support is deferred with an
//! [`MiliError::UnsupportedDir(1)`] at header parse, this module's
//! only job in Phase 1 is to enumerate any separate TI files that
//! *would* be present alongside the database and return them — which
//! is always an empty list for the supported v2/v3 case. Keeping the
//! function here means [`crate::family`] can call it unconditionally
//! once it lands.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Filename pattern for v1 TI files.
///
/// `<root>_TI_<base26>` where base26 = `A, B, …, Z, AA, AB, …`
/// (`reference/mili/src/mili_util.c:908-911, 921-946`).
fn ti_filename(root: &Path, index: u32) -> PathBuf {
    let stem = root.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let mut name = String::with_capacity(stem.len() + 8);
    name.push_str(stem);
    name.push_str("_TI_");
    append_base26_upper(&mut name, index);
    root.with_file_name(name)
}

fn append_base26_upper(out: &mut String, num: u32) {
    // Re-implementation of `to_base26(num, TRUE, …)` from
    // `reference/mili/src/mili_util.c:921-946`. The "zero" digit is
    // 'A', so 0 → "A", 25 → "Z", 26 → "BA", etc. — the high digit is
    // padded out to the number of base-26 digits needed.
    let mut pwr: u32 = 0;
    let mut n: u32 = 1;
    loop {
        let next_n = n.saturating_mul(26);
        if next_n.saturating_sub(1) >= num {
            break;
        }
        n = next_n;
        pwr += 1;
    }
    let mut remaining = num;
    for _ in 0..=pwr {
        let mult = remaining / n;
        out.push((b'A' + mult as u8) as char);
        remaining -= mult * n;
        n /= 26;
        if n == 0 {
            break;
        }
    }
}

/// Locate any `<root>_TI_*` files alongside `root` and return their
/// paths in `[A, B, C, …]` order. In a v2+ database (everything we
/// currently support) this is always empty.
///
/// Errors only on I/O — missing files are not an error, they end the
/// scan.
pub fn enumerate_ti_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut idx: u32 = 0;
    loop {
        let path = ti_filename(root, idx);
        match path.try_exists() {
            Ok(true) => {
                out.push(path);
                idx = idx
                    .checked_add(1)
                    .expect("TI file count exceeded u32 — corrupt database");
            }
            Ok(false) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn base26_matches_c_to_base26() {
        let cases = [
            (0, "A"),
            (1, "B"),
            (25, "Z"),
            (26, "BA"),
            (51, "BZ"),
            (52, "CA"),
        ];
        for (n, expected) in cases {
            let mut s = String::new();
            append_base26_upper(&mut s, n);
            assert_eq!(s, expected, "base26({n})");
        }
    }

    #[test]
    fn ti_filename_format() {
        let p = ti_filename(Path::new("/tmp/run.pltA"), 0);
        assert_eq!(p, Path::new("/tmp/run.pltA_TI_A"));
        let p = ti_filename(Path::new("/tmp/run.pltA"), 26);
        assert_eq!(p, Path::new("/tmp/run.pltA_TI_BA"));
    }

    #[test]
    fn enumerate_returns_empty_for_v2_corpus() {
        // basic1 is a v3 fixture — no TI files exist on disk.
        let path = Path::new("../../reference/mili-python/tests/data/serial/basic1/basic1.pltA");
        if !path.exists() {
            // Submodule not initialized; skip rather than fail CI in
            // environments where reference data is absent.
            return;
        }
        let files = enumerate_ti_files(path).unwrap();
        assert!(files.is_empty(), "v3 fixture should ship no TI files");
    }
}
