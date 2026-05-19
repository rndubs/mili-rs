//! Cross-impl parity vs. the upstream
//! `mili.miliinternal._MiliInternal` oracle for the derived-variable
//! *enumeration* surface (`supported_derived_variables`,
//! `derived_variables_of_class`, `classes_of_derived_variable`), swept
//! across the full serial corpus.
//!
//! These three compose the already parity-gated primal reshapes with
//! the static `DERIVED_REGISTRY`; this binary is what validates the
//! 55-entry registry transcription (`reshape.rs`) — every entry's
//! `primals` / `alternate_primals` / `primals_class` / `only_sclasses`
//! is exercised transitively through every class of every fixture.
//! `supported_derived_variables` / `derived_variables_of_class` are
//! **exact** (deterministic registry insertion order on both sides);
//! `classes_of_derived_variable` is **set-equal** — its class order is
//! inherited from `mesh_object_classes`, an orthogonal reshape whose
//! ordering vs the oracle is its own (un-order-gated) concern. Gated
//! on the `parity` feature; skip-not-fail when the corpus or `mili` is
//! absent (CLAUDE.md skip-on-absent discipline).

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;

use mili_rs::{Database, MeshId};

struct Fx {
    rel_dir: &'static str,
    a_file: &'static str,
    base: &'static str,
}

const CORPUS: &[Fx] = &[
    Fx {
        rel_dir: "beam_udi",
        a_file: "beam_udi.pltA",
        base: "beam_udi.plt",
    },
    Fx {
        rel_dir: "d3samp4",
        a_file: "d3samp4.pltA",
        base: "d3samp4.plt",
    },
    Fx {
        rel_dir: "dbl_nodtang",
        a_file: "dblplt000A",
        base: "dblplt000",
    },
    Fx {
        rel_dir: "fdamp1",
        a_file: "fdamp1.pltA",
        base: "fdamp1.plt",
    },
    Fx {
        rel_dir: "labeling",
        a_file: "dblplt003A",
        base: "dblplt003",
    },
    Fx {
        rel_dir: "mstate",
        a_file: "d3samp6.plt_cA",
        base: "d3samp6.plt_c",
    },
    Fx {
        rel_dir: "rigid_body_1",
        a_file: "rigid_body1.pltA",
        base: "rigid_body1.plt",
    },
    Fx {
        rel_dir: "sstate",
        a_file: "d3samp6.pltA",
        base: "d3samp6.plt",
    },
    Fx {
        rel_dir: "tet",
        a_file: "tet1_t4.pltA",
        base: "tet1_t4.plt",
    },
    Fx {
        rel_dir: "vrt_BS",
        a_file: "vrt_BS.pltA",
        base: "vrt_BS.plt",
    },
];

fn oracle<'py>(py: Python<'py>, fx: &Fx) -> PyResult<Bound<'py, PyAny>> {
    let dir = corpus_path(&["serial", fx.rel_dir]);
    let m = py.import_bound("mili.miliinternal")?;
    m.getattr("_MiliInternal")?
        .call1((dir.to_str().expect("utf8"), fx.base))
}

fn pylist_str(o: &Bound<'_, PyAny>) -> Vec<String> {
    o.extract::<Vec<String>>().expect("list[str]")
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn parity_derived_enum_corpus() {
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    Python::with_gil(|py| {
        for fx in CORPUS {
            let a = corpus_path(&["serial", fx.rel_dir, fx.a_file]);
            if !a.exists() {
                eprintln!("skip: {} absent", fx.rel_dir);
                continue;
            }
            let db = Database::open(&a).expect("open rust db");
            let mesh = db
                .meshes()
                .meshes()
                .map(|m| m.id)
                .min()
                .unwrap_or(MeshId(0));
            let ora = oracle(py, fx).expect("open oracle");
            let tag = fx.rel_dir;

            // ---- supported_derived_variables (registry keys) ----
            let sup = db.supported_derived_variables();
            let osup = pylist_str(&ora.call_method0("supported_derived_variables").unwrap());
            assert_eq!(sup, osup, "{tag}: supported_derived_variables");

            // ---- derived_variables_of_class, every class ----
            for class_name in db.class_names(mesh) {
                let r = db.derived_variables_of_class(mesh, &class_name);
                let o = pylist_str(
                    &ora.call_method1("derived_variables_of_class", (&class_name,))
                        .unwrap(),
                );
                assert_eq!(r, o, "{tag}: derived_variables_of_class({class_name})");
            }

            // ---- classes_of_derived_variable, every derived name ----
            // Set-equal: the per-class *order* is inherited verbatim
            // from `mesh_object_classes` (CLASS_DEF order), an
            // orthogonal reshape whose ordering vs the oracle is its
            // own concern and is not order-gated by `parity_reshape`
            // (mstate diverges). What this registry owns — *which*
            // classes pass the primal/`only_sclasses` gates — is
            // exactly the set, so the set is the faithful contract
            // here (the `parity_reshape` "set(...) artifact" rule).
            for name in &sup {
                let r = db
                    .classes_of_derived_variable(mesh, name)
                    .expect("known derived name resolves");
                let o = pylist_str(
                    &ora.call_method1("classes_of_derived_variable", (name,))
                        .unwrap(),
                );
                assert_eq!(
                    sorted(r),
                    sorted(o),
                    "{tag}: classes_of_derived_variable({name})"
                );
            }
        }
    });
}
