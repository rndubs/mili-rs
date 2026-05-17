//! Corpus-coverage hard gate — the Rust-side analogue of the milox
//! `test_redirect_coverage_is_exhaustive` guard (PR #39 / m4.md
//! decision 25).
//!
//! The parity / fixture suites (`parity_corpus.rs`, `parity_xmilics.rs`,
//! `smoke_full_corpus.rs`, the `*_fixtures.rs`) all *early-return
//! instead of failing* when their fixture family is absent — so a bare
//! `cargo test` reads green while CI catches the regression
//! (`CLAUDE.md` § "Parity / fixture tests skip-on-absent"). With every
//! family individually skip-on-absent, the remaining silent-rot path
//! is: a fixture family that quietly disappears (an emptied / partially
//! checked-out submodule) or a brand-new upstream family that no suite
//! references — either way zero failures and silently shrunk coverage.
//!
//! This guard closes that. It is `parity`-feature-gated, so it only
//! runs in CI's `test-parity` job, which runs `scripts/setup-parity.sh`
//! (inits both submodules) *before* `cargo test --features parity` —
//! i.e. when the parity environment is present, every accounted family
//! MUST be on disk, and every on-disk family MUST be accounted. A bare
//! local `cargo test --features parity` without the submodule still
//! skips-not-fails (each submodule root checked independently), exactly
//! the existing convention.
//!
//! The accounted sets below mirror `smoke_full_corpus.rs::corpora()`
//! (the canonical 14 serial + 2 parallel + 3 v3 + 2 th + 9 xmilics
//! family manifest). When a future upstream submodule bump adds a
//! corpus directory, this guard fails until it is consciously wired
//! into the suites + listed here (or added to `EXCLUDED` with a
//! concrete reason — like the milox `EXCLUDED` set).

#![cfg(feature = "parity")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every `reference/mili-python/tests/data/serial/<dir>` family the
/// suites exercise (== `smoke_full_corpus`'s 14 serial rows).
const SERIAL: &[&str] = &[
    "basic1",
    "beam_udi",
    "d3samp4",
    "dbl_nodtang",
    "dir_version_2",
    "fdamp1",
    "labeling",
    "mstate",
    "rigid_body_1",
    "solids014",
    "sstate",
    "tet",
    "vecarray",
    "vrt_BS",
];

/// `reference/mili-python/tests/data/parallel/<dir>` families.
const PARALLEL: &[&str] = &["basic1", "d3samp6"];

/// `reference/mili-python/tests/data/v3/<dir>` families.
const V3: &[&str] = &["no_tfile", "parallel_t", "serial_t"];

/// `reference/mili-python/tests/data/th/<dir>` families.
const TH: &[&str] = &["serial", "parallel"];

/// `reference/mili/test/xmilics/<dir>` C-library families (the 9
/// advertised in `smoke_full_corpus`).
const XMILICS: &[&str] = &[
    "bar1",
    "bar5",
    "basic2",
    "cylinder",
    "cylinder_4hex",
    "d3samp6",
    "d3samp6_tfile",
    "ml40",
    "shell_mat2",
];

/// On-disk directory names that are NOT corpus families and so are
/// intentionally not accounted (the documented-non-gap mechanism,
/// mirroring the milox `EXCLUDED` set). `image_baselines` is a sibling
/// of `serial/`/`parallel/`/… under `data/` (matplotlib PNG baselines
/// for the excluded upstream `test_plotting`, PR #39), never enumerated
/// here because we only descend the specific family parents — kept as a
/// commented anchor for future additions.
const EXCLUDED: &[&str] = &[];

/// Directory names directly under `dir`, ignoring files.
fn subdirs(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Some(name) = entry.file_name().to_str() {
                out.insert(name.to_owned());
            }
        }
    }
    out
}

/// Assert the family directories actually present under `parent` equal
/// exactly `accounted ∪ EXCLUDED`, and that each accounted family dir
/// is non-empty (catches a hollowed-out / partially-fetched submodule
/// that the per-family skip-on-absent would otherwise hide).
fn assert_group_exhaustive(group: &str, parent: &Path, accounted: &[&str]) {
    let present = subdirs(parent);
    let accounted_set: BTreeSet<String> = accounted.iter().map(|s| (*s).to_owned()).collect();
    let excluded_set: BTreeSet<String> = EXCLUDED.iter().map(|s| (*s).to_owned()).collect();

    let uncovered: Vec<&String> = present
        .iter()
        .filter(|d| !accounted_set.contains(*d) && !excluded_set.contains(*d))
        .collect();
    assert!(
        uncovered.is_empty(),
        "{group}: upstream corpus family/families present on disk but \
         neither exercised+accounted nor EXCLUDED — consciously wire \
         into the parity/smoke suites and add here (or EXCLUDED with a \
         concrete reason): {uncovered:?} (under {})",
        parent.display()
    );

    let missing: Vec<&str> = accounted
        .iter()
        .copied()
        .filter(|d| !present.contains(*d))
        .collect();
    assert!(
        missing.is_empty(),
        "{group}: accounted corpus family/families absent under {} — \
         the parity environment is present (parity feature + \
         setup-parity.sh) so this is silent coverage loss, not a \
         benign skip: {missing:?}",
        parent.display()
    );

    for fam in accounted {
        let famdir = parent.join(fam);
        let nonempty = std::fs::read_dir(&famdir)
            .map(|mut rd| rd.next().is_some())
            .unwrap_or(false);
        assert!(
            nonempty,
            "{group}: corpus family '{fam}' directory is empty/unreadable \
             ({}) — a partially-fetched submodule silently drops its \
             parity coverage",
            famdir.display()
        );
    }
}

/// `reference/mili-python` submodule: serial / parallel / v3 / th.
#[test]
fn mili_python_corpus_coverage_is_exhaustive() {
    let data = workspace_root().join("reference/mili-python/tests/data");
    if !data.join("serial").is_dir() {
        // reference/mili-python not checked out — skip-not-fail, the
        // existing convention (a bare `cargo test --features parity`
        // without setup-parity.sh).
        eprintln!("skip: reference/mili-python submodule absent");
        return;
    }
    assert_group_exhaustive("serial", &data.join("serial"), SERIAL);
    assert_group_exhaustive("parallel", &data.join("parallel"), PARALLEL);
    assert_group_exhaustive("v3", &data.join("v3"), V3);
    assert_group_exhaustive("th", &data.join("th"), TH);
}

/// `reference/mili` submodule: the C-library xmilics families. Checked
/// independently so a checkout with only one submodule still partially
/// guards without a false failure.
#[test]
fn xmilics_corpus_coverage_is_exhaustive() {
    let xmilics = workspace_root().join("reference/mili/test/xmilics");
    if !xmilics.is_dir() {
        eprintln!("skip: reference/mili submodule absent");
        return;
    }
    assert_group_exhaustive("xmilics", &xmilics, XMILICS);
}
