//! M4-followup Phase G — cross-impl parity vs. the upstream
//! `mili.miliinternal._MiliInternal` oracle for every primal-only
//! reshape, swept across the full serial corpus.
//!
//! For each fixture this opens the Rust core and the upstream
//! `_MiliInternal(dir, base)` and asserts the reshape methods are
//! **bit-exact** (set-equal where upstream's order is a `set(...)`
//! artifact). Gated on the `parity` feature; skip-not-fail when the
//! corpus or `mili` package is absent (mirrors the other parity
//! binaries / CLAUDE.md).

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;
use pyo3::types::PyList;

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

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn oracle<'py>(py: Python<'py>, fx: &Fx) -> PyResult<Bound<'py, PyAny>> {
    let dir = corpus_path(&["serial", fx.rel_dir]);
    let m = py.import_bound("mili.miliinternal")?;
    m.getattr("_MiliInternal")?
        .call1((dir.to_str().expect("utf8"), fx.base))
}

/// Upstream IntEnum / numpy scalar → i64.
fn as_i64(o: &Bound<'_, PyAny>) -> i64 {
    if let Ok(v) = o.extract::<i64>() {
        return v;
    }
    o.getattr("value")
        .and_then(|v| v.extract::<i64>())
        .expect("int-like")
}

fn pylist_str(o: &Bound<'_, PyAny>) -> Vec<String> {
    o.extract::<Vec<String>>().expect("list[str]")
}

fn pyarr_i32(o: &Bound<'_, PyAny>) -> Vec<i32> {
    // numpy array / list → Vec<i32> via .tolist() when present.
    if let Ok(tl) = o.call_method0("tolist") {
        return tl.extract::<Vec<i32>>().expect("int list");
    }
    o.extract::<Vec<i32>>().expect("int list")
}

#[test]
fn parity_reshape_corpus() {
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

            // ---- srec_fmt_qty ----
            assert_eq!(
                i64::from(db.srec_fmt_qty()),
                as_i64(&ora.call_method0("srec_fmt_qty").unwrap()),
                "{tag}: srec_fmt_qty"
            );

            // ---- metadata ----
            let md = db.metadata().unwrap();
            let omd = ora.call_method0("metadata").unwrap();
            for (k, v) in [
                ("code_name", md.code_name),
                ("username", md.username),
                ("job_id", md.job_id),
                ("date", md.date),
                ("host_name", md.host_name),
                ("library_version", md.library_version),
            ] {
                let ov: String = omd.get_item(k).unwrap().extract().unwrap();
                assert_eq!(v, ov, "{tag}: metadata.{k}");
            }
            let onp: i64 = as_i64(&omd.get_item("nprocs").unwrap());
            assert_eq!(i64::from(md.nprocs), onp, "{tag}: metadata.nprocs");

            // ---- class-keyed reshapes ----
            let class_names = db.class_names(mesh);
            let mocs = db.mesh_object_classes(mesh).unwrap();
            let omoc = ora.call_method0("mesh_object_classes").unwrap();
            assert_eq!(mocs.len(), class_names.len(), "{tag}: moc count");
            for c in &mocs {
                let oc = omoc.get_item(&c.short_name).unwrap();
                assert_eq!(
                    i64::from(c.sclass),
                    as_i64(&oc.getattr("sclass").unwrap()),
                    "{tag}: {} sclass",
                    c.short_name
                );
                assert_eq!(
                    c.long_name,
                    oc.getattr("long_name")
                        .unwrap()
                        .extract::<String>()
                        .unwrap(),
                    "{tag}: {} long_name",
                    c.short_name
                );
                assert_eq!(
                    i64::from(c.elem_qty),
                    as_i64(&oc.getattr("elem_qty").unwrap()),
                    "{tag}: {} elem_qty",
                    c.short_name
                );
                assert_eq!(
                    c.idents_exist,
                    oc.getattr("idents_exist")
                        .unwrap()
                        .extract::<bool>()
                        .unwrap(),
                    "{tag}: {} idents_exist",
                    c.short_name
                );
            }

            for cn in &class_names {
                assert_eq!(
                    i64::from(db.superclass_code(mesh, cn).unwrap_or(-1)),
                    as_i64(
                        &ora.call_method1("superclass_from_class_name", (cn,))
                            .unwrap()
                    ),
                    "{tag}: superclass_from_class_name({cn})"
                );
                // state_variables_of_class — exact list/order.
                let rs = db.state_variables_of_class(mesh, cn).unwrap_or_default();
                let os = pylist_str(&ora.call_method1("state_variables_of_class", (cn,)).unwrap());
                assert_eq!(rs, os, "{tag}: state_variables_of_class({cn})");
                // materials_of_class_name / parts_of_class_name.
                let rm = db.materials_of_class_name(mesh, cn).unwrap().unwrap();
                let om = pyarr_i32(&ora.call_method1("materials_of_class_name", (cn,)).unwrap());
                assert_eq!(rm, om, "{tag}: materials_of_class_name({cn})");
                let rp = db.parts_of_class_name(mesh, cn).unwrap().unwrap();
                let op = pyarr_i32(&ora.call_method1("parts_of_class_name", (cn,)).unwrap());
                assert_eq!(rp, op, "{tag}: parts_of_class_name({cn})");
            }

            // ---- subrecords ----
            let subs = db.subrecords(mesh);
            let osubs = ora.call_method0("subrecords").unwrap();
            let osubs = osubs.downcast::<PyList>().unwrap();
            assert_eq!(subs.len(), osubs.len(), "{tag}: subrecord count");
            for (s, os) in subs.iter().zip(osubs.iter()) {
                assert_eq!(
                    s.name,
                    os.getattr("name").unwrap().extract::<String>().unwrap(),
                    "{tag}: subrec name"
                );
                assert_eq!(
                    s.class_name,
                    os.getattr("class_name")
                        .unwrap()
                        .extract::<String>()
                        .unwrap(),
                    "{tag}: {} subrec class_name",
                    s.name
                );
                assert_eq!(
                    i64::from(s.superclass),
                    as_i64(&os.getattr("superclass").unwrap()),
                    "{tag}: {} subrec superclass",
                    s.name
                );
                assert_eq!(
                    i64::from(s.organization),
                    as_i64(&os.getattr("organization").unwrap()),
                    "{tag}: {} subrec organization",
                    s.name
                );
                assert_eq!(
                    i64::from(s.qty_svars),
                    as_i64(&os.getattr("qty_svars").unwrap()),
                    "{tag}: {} subrec qty_svars",
                    s.name
                );
                assert_eq!(
                    s.svar_names,
                    pylist_str(&os.getattr("svar_names").unwrap()),
                    "{tag}: {} subrec svar_names",
                    s.name
                );
                let ob: Vec<i64> = os
                    .getattr("ordinal_blocks")
                    .unwrap()
                    .call_method0("tolist")
                    .unwrap()
                    .extract()
                    .unwrap();
                assert_eq!(s.ordinal_blocks, ob, "{tag}: {} ordinal_blocks", s.name);
            }

            // ---- state_variables ----
            let svs = db.state_variables();
            let osv = ora.call_method0("state_variables").unwrap();
            assert_eq!(
                svs.len(),
                osv.len().unwrap(),
                "{tag}: state_variables count"
            );
            for v in &svs {
                let o = osv.get_item(&v.name).unwrap();
                assert_eq!(
                    v.title,
                    o.getattr("title").unwrap().extract::<String>().unwrap(),
                    "{tag}: {} title",
                    v.name
                );
                assert_eq!(
                    i64::from(v.agg_type),
                    as_i64(&o.getattr("agg_type").unwrap()),
                    "{tag}: {} agg_type",
                    v.name
                );
                assert_eq!(
                    i64::from(v.data_type),
                    as_i64(&o.getattr("data_type").unwrap()),
                    "{tag}: {} data_type",
                    v.name
                );
                assert_eq!(
                    i64::from(v.list_size),
                    as_i64(&o.getattr("list_size").unwrap()),
                    "{tag}: {} list_size",
                    v.name
                );
                assert_eq!(
                    i64::from(v.order),
                    as_i64(&o.getattr("order").unwrap()),
                    "{tag}: {} order",
                    v.name
                );
                let od: Vec<i32> = {
                    let d = o.getattr("dims").unwrap();
                    d.call_method0("tolist")
                        .and_then(|t| t.extract())
                        .or_else(|_| d.extract())
                        .unwrap()
                };
                assert_eq!(v.dims, od, "{tag}: {} dims", v.name);
                assert_eq!(
                    v.comp_names,
                    pylist_str(&o.getattr("comp_names").unwrap()),
                    "{tag}: {} comp_names",
                    v.name
                );
                assert_eq!(
                    v.containing_svar_names,
                    pylist_str(&o.getattr("containing_svar_names").unwrap()),
                    "{tag}: {} containing_svar_names",
                    v.name
                );

                // classes_of_state_variable — upstream `list(set(...))`,
                // so compare as a set.
                let rc = sorted(db.classes_of_state_variable(&v.name).unwrap_or_default());
                let oc = sorted(pylist_str(
                    &ora.call_method1("classes_of_state_variable", (&v.name,))
                        .unwrap(),
                ));
                assert_eq!(rc, oc, "{tag}: classes_of_state_variable({})", v.name);

                // components_of_vector_svar for vector/vec_array svars.
                if v.agg_type == 1 || v.agg_type == 3 {
                    let rcv = db.components_of_vector_svar(&v.name).unwrap();
                    let ocv = pylist_str(
                        &ora.call_method1("components_of_vector_svar", (&v.name,))
                            .unwrap(),
                    );
                    assert_eq!(rcv, ocv, "{tag}: components_of_vector_svar({})", v.name);
                }

                // int_points_of_state_variable across every class.
                for cn in &class_names {
                    let r = db
                        .int_points_of_state_variable(mesh, &v.name, cn)
                        .unwrap_or_default();
                    let o = pyarr_i32(
                        &ora.call_method1("int_points_of_state_variable", (&v.name, cn))
                            .unwrap(),
                    );
                    assert_eq!(r, o, "{tag}: int_points_of_state_variable({},{cn})", v.name);
                }
            }

            // ---- queriable_svars (all 3 flag combos) ----
            for (vo, si) in [(false, false), (true, false), (true, true)] {
                let r = db.queriable_svars(vo, si);
                let o = pylist_str(&ora.call_method1("queriable_svars", (vo, si)).unwrap());
                assert_eq!(r, o, "{tag}: queriable_svars({vo},{si})");
            }

            // ---- state_variable_titles ----
            let rt = db.state_variable_titles();
            let ot = ora.call_method0("state_variable_titles").unwrap();
            for (k, val) in &rt {
                let ov: String = ot.get_item(k).unwrap().extract().unwrap();
                assert_eq!(val, &ov, "{tag}: state_variable_titles[{k}]");
            }

            // ---- parameters / parameter ----
            let rp = db.parameters().unwrap();
            let op = ora.call_method0("parameters").unwrap();
            let rkeys = sorted(rp.iter().map(|(k, _)| k.clone()).collect());
            let okeys = sorted(
                op.downcast::<pyo3::types::PyDict>()
                    .unwrap()
                    .keys()
                    .extract()
                    .unwrap(),
            );
            assert_eq!(rkeys, okeys, "{tag}: parameters keys");
        }
    });
}
