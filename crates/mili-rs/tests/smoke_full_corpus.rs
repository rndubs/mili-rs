//! `reader.c`-style smoke coverage: for every fixture in the two
//! reference corpora, open the family, enumerate classes, svars,
//! states, and run a couple of representative queries. No oracle —
//! just "doesn't panic, sizes are coherent."
//!
//! The point is to catch the long tail across the 14 serial + 2
//! parallel + 3 v3 mili-python fixtures and the 9 xmilics + extras
//! C-library fixtures. Each fixture is opened via [`DatabaseSet`] so
//! the multi-fragment path is also exercised whenever the family is
//! MPI-segmented.
//!
//! Skip-on-absent: if neither submodule is checked out, every fixture
//! is reported as skipped and the test passes. Run after
//! `git submodule update --init reference/mili-python reference/mili`
//! for full coverage.

use std::path::{Path, PathBuf};

use mili_rs::{DatabaseSet, MeshId, MiliError, QueryArgs};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// `(label, family-base path)` rows. The label is just for diagnostics.
fn corpora() -> Vec<(&'static str, PathBuf)> {
    let root = workspace_root();
    let mili_python = root.join("reference/mili-python/tests/data");
    let mili_c = root.join("reference/mili/test/xmilics");
    vec![
        // mili-python serial fixtures (14)
        (
            "mp/serial/basic1",
            mili_python.join("serial/basic1/basic1.plt"),
        ),
        (
            "mp/serial/beam_udi",
            mili_python.join("serial/beam_udi/beam_udi.plt"),
        ),
        (
            "mp/serial/d3samp4",
            mili_python.join("serial/d3samp4/d3samp4.plt"),
        ),
        (
            "mp/serial/dbl_nodtang",
            mili_python.join("serial/dbl_nodtang/dblplt"),
        ),
        (
            "mp/serial/dir_version_2",
            mili_python.join("serial/dir_version_2/dblplt"),
        ),
        (
            "mp/serial/fdamp1",
            mili_python.join("serial/fdamp1/fdamp1.plt"),
        ),
        (
            "mp/serial/labeling",
            mili_python.join("serial/labeling/dblplt"),
        ),
        (
            "mp/serial/mstate",
            mili_python.join("serial/mstate/d3samp6.plt_c"),
        ),
        (
            "mp/serial/rigid_body_1",
            mili_python.join("serial/rigid_body_1/rigid_body1.plt"),
        ),
        (
            "mp/serial/solids014",
            mili_python.join("serial/solids014/solids014_dblplt"),
        ),
        (
            "mp/serial/sstate",
            mili_python.join("serial/sstate/d3samp6.plt"),
        ),
        ("mp/serial/tet", mili_python.join("serial/tet/tet1_t4.plt")),
        (
            "mp/serial/vecarray",
            mili_python.join("serial/vecarray/shell_mat15/shell_mat15.plt"),
        ),
        (
            "mp/serial/vrt_BS",
            mili_python.join("serial/vrt_BS/vrt_BS.plt"),
        ),
        // mili-python parallel (2)
        (
            "mp/parallel/basic1",
            mili_python.join("parallel/basic1/basic1.plt"),
        ),
        (
            "mp/parallel/d3samp6",
            mili_python.join("parallel/d3samp6/d3samp6.plt"),
        ),
        // mili-python v3 (3)
        (
            "mp/v3/no_tfile",
            mili_python.join("v3/no_tfile/d3samp6.plt"),
        ),
        (
            "mp/v3/parallel_t",
            mili_python.join("v3/parallel_t/d3samp6.plt"),
        ),
        (
            "mp/v3/serial_t",
            mili_python.join("v3/serial_t/d3samp6.plt"),
        ),
        // mili-python time-history fixtures
        ("mp/th/serial", mili_python.join("th/serial/d3samp6.th")),
        ("mp/th/parallel", mili_python.join("th/parallel/d3samp6.th")),
        // C-library xmilics (9 advertised + extras the writer happens to emit)
        ("c/xmilics/bar1", mili_c.join("bar1/bar1.plt")),
        ("c/xmilics/bar5", mili_c.join("bar5/bar5.plt")),
        ("c/xmilics/basic2", mili_c.join("basic2/basic2.plt")),
        ("c/xmilics/cylinder", mili_c.join("cylinder/cylinder.plt")),
        (
            "c/xmilics/cylinder_4hex",
            mili_c.join("cylinder_4hex/cylinder.plt"),
        ),
        ("c/xmilics/d3samp6", mili_c.join("d3samp6/d3samp6.plt")),
        (
            "c/xmilics/d3samp6_tfile",
            mili_c.join("d3samp6_tfile/d3samp6.plt"),
        ),
        ("c/xmilics/ml40", mili_c.join("ml40/ml40.plt")),
        (
            "c/xmilics/shell_mat2",
            mili_c.join("shell_mat2/shell_mat2.plt"),
        ),
    ]
}

/// Walk a single family and assert internal coherence.
///
/// Returns `Some(skip_reason)` if the family's directory isn't on disk
/// (submodule not checked out). All other failures panic so the
/// failing fixture is named in the panic.
fn smoke_one(label: &str, base: &Path) -> Option<String> {
    let parent = base.parent().expect("base path has parent");
    if !parent.exists() {
        return Some(format!("skip {label}: corpus dir absent"));
    }
    let set = DatabaseSet::open(base)
        .unwrap_or_else(|e| panic!("{label}: DatabaseSet::open({}) -> {e}", base.display()));

    let state_count = set.state_count();
    let times = set.times();
    assert_eq!(
        times.len(),
        state_count,
        "{label}: times/state_count mismatch"
    );

    // Every fragment agrees on the time axis (checked in
    // DatabaseSet::open). Enumerate classes + svars on each fragment
    // and confirm the labels accessor doesn't panic on any class /
    // mesh combo.
    let mut total_classes = 0usize;
    let mut total_svars = 0usize;
    for (rank, frag) in set.fragments().iter().enumerate() {
        for mesh in frag.meshes().meshes() {
            for class in mesh.classes() {
                total_classes += 1;
                let res = frag.labels(mesh.id, &class.short_name);
                assert!(
                    res.is_ok(),
                    "{label}/rank{rank}: labels({:?}, {:?}) -> {:?}",
                    mesh.id,
                    class.short_name,
                    res.err()
                );
            }
        }
        total_svars += frag.svars().iter().count();
    }
    assert!(
        total_classes > 0,
        "{label}: no classes discovered across {} fragments",
        set.fragment_count()
    );
    assert!(
        total_svars > 0,
        "{label}: no svars discovered across {} fragments",
        set.fragment_count()
    );

    // Try a representative query that should exist in nearly every
    // mili family — `nodpos` on the `node` class. If a fixture
    // doesn't have it, treat as benign skip.
    let states_to_sample: Vec<usize> = if state_count == 0 {
        Vec::new()
    } else if state_count <= 4 {
        (0..state_count).collect()
    } else {
        vec![0, state_count / 2, state_count - 1]
    };
    if states_to_sample.is_empty() {
        return None;
    }
    let args = QueryArgs {
        svar: "nodpos",
        class: "node",
        labels: None,
        states: &states_to_sample,
        materials: None,
        ips: None,
    };
    match set.query(&args) {
        Ok(r) => {
            assert_eq!(r.state_count, states_to_sample.len());
            assert!(r.atoms_per_label > 0, "{label}: zero atoms_per_label");
            assert_eq!(
                r.values.len(),
                r.state_count * r.labels.len() * r.atoms_per_label,
                "{label}: query value count not coherent"
            );
        }
        Err(
            MiliError::NoMatchingSubrec { .. }
            | MiliError::UnknownClass(_)
            | MiliError::UnknownSvar(_),
        ) => {
            // Benign — the fixture just doesn't expose this svar/class
            // combo. Common for `th/*` fixtures (which only carry
            // glob-class state results).
        }
        Err(MiliError::Io(ref io)) if io.kind() == std::io::ErrorKind::NotFound => {
            // Benign for fixtures that ship only the .A file with no
            // state files on disk (e.g. `serial/dir_version_2`, which
            // ships `dblplt2009A` alone for v2-directory parsing).
        }
        Err(e) => {
            panic!("{label}: nodpos query failed unexpectedly: {e}");
        }
    }

    // Also exercise DatabaseSet::labels for the node class — exercises
    // the merge path on multi-fragment families.
    let _ = set
        .labels(MeshId(0), "node")
        .unwrap_or_else(|e| panic!("{label}: DatabaseSet::labels(MeshId(0), node) -> {e}"));

    None
}

#[test]
fn smoke_walks_every_corpus_family() {
    let mut skipped: Vec<String> = Vec::new();
    let mut ran = 0usize;
    for (label, base) in corpora() {
        match smoke_one(label, &base) {
            Some(reason) => skipped.push(reason),
            None => ran += 1,
        }
    }
    eprintln!(
        "smoke_full_corpus: ran {ran}, skipped {} (submodule(s) absent)",
        skipped.len()
    );
    if ran == 0 && !skipped.is_empty() {
        eprintln!("(every fixture skipped — submodules not checked out)");
    }
}
