//! Parameter decode parity against the reference corpus.
//!
//! Skips when the submodule is absent so partial checkouts stay green.

use std::fs;
use std::path::{Path, PathBuf};

use mili_rs::{AggType, DataType, Directory, Header, ParamTable, ParamValue, ScalarValue};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data")
        .join("serial")
}

fn read_a_file(rel: &[&str]) -> Option<Vec<u8>> {
    let mut p = corpus_root();
    for c in rel {
        p = p.join(c);
    }
    fs::read(&p).ok()
}

fn open(rel: &[&str]) -> Option<(Vec<u8>, Header, Directory, ParamTable)> {
    let bytes = read_a_file(rel)?;
    let header = Header::parse(&bytes).unwrap();
    let dir = Directory::parse(&bytes, &header).unwrap();
    let table = ParamTable::build(&dir);
    Some((bytes, header, dir, table))
}

#[test]
fn basic1_mesh_dimensions_is_3() {
    let Some((bytes, header, dir, table)) = open(&["basic1", "basic1.pltA"]) else {
        eprintln!("skip: basic1 absent");
        return;
    };
    let idx = table.get("mesh dimensions").expect("missing scalar");
    let v = ParamValue::decode(&bytes, &dir.entries[idx], header).unwrap();
    match v {
        ParamValue::Scalar(ScalarValue::I32(3)) => {}
        other => panic!("expected i32(3), got {other:?}"),
    }
}

#[test]
fn basic1_states_per_file_decodes_as_i32() {
    let Some((bytes, header, dir, table)) = open(&["basic1", "basic1.pltA"]) else {
        return;
    };
    // The basic1 writer leaves this scalar at the sentinel `0` (meaning
    // "use the default 10,000,000" — `reference/mili/src/mili.c:2862`).
    // We only verify shape here; the family layer interprets the zero.
    let idx = table.get("states per file").expect("missing scalar");
    let v = ParamValue::decode(&bytes, &dir.entries[idx], header).unwrap();
    assert!(matches!(v, ParamValue::Scalar(ScalarValue::I32(_))));
}

#[test]
fn basic1_lib_version_is_nonempty_string() {
    let Some((bytes, header, dir, table)) = open(&["basic1", "basic1.pltA"]) else {
        return;
    };
    let idx = table.get("lib version").expect("missing string param");
    let v = ParamValue::decode(&bytes, &dir.entries[idx], header).unwrap();
    match v {
        ParamValue::String(s) => {
            assert!(!s.is_empty(), "lib version is empty");
            assert!(
                s.chars().all(|c| c.is_ascii() && !c.is_control()),
                "lib version contains non-printable bytes: {s:?}"
            );
        }
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn basic1_ti_param_node_labels_array() {
    let Some((bytes, header, dir, table)) = open(&["basic1", "basic1.pltA"]) else {
        return;
    };
    let name = "Node Labels[/Mesh-0/Sname-node/Scls-M_NODE/Mat--1/]";
    let idx = table.get(name).expect("missing TI node-labels entry");
    let entry = &dir.entries[idx];
    assert_eq!(entry.modifier1, DataType::Int as i64);
    assert_eq!(entry.modifier2, AggType::Array as i64);
    let v = ParamValue::decode(&bytes, entry, header).unwrap();
    match v {
        ParamValue::Array(a) => {
            assert_eq!(a.data_type, DataType::Int);
            assert_eq!(a.dims.len(), 1, "node label array is 1-d");
            // basic1 has 1400 nodes (Nodes m2=1400 in the directory).
            assert_eq!(a.atoms, 1400);
            assert_eq!(a.data.len(), 1400 * 4);
            // First label is the standard 1-based id 1.
            let first = i32::from_le_bytes(a.data[0..4].try_into().unwrap());
            assert_eq!(first, 1);
        }
        other => panic!("expected array, got {other:?}"),
    }
}

#[test]
fn dbl_nodtang_mat_name_1() {
    let Some((bytes, header, dir, table)) = open(&["dbl_nodtang", "dblplt000A"]) else {
        eprintln!("skip: dbl_nodtang absent");
        return;
    };
    // MAT_NAME_1 is a TI_PARAM string (modifier1=M_STRING, modifier2=0).
    let idx = table.get("MAT_NAME_1").expect("missing MAT_NAME_1");
    let v = ParamValue::decode(&bytes, &dir.entries[idx], header).unwrap();
    match v {
        ParamValue::String(s) => {
            assert!(!s.is_empty());
            assert!(s.chars().all(|c| c.is_ascii() && !c.is_control()));
        }
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn param_table_contains_ti_params_in_v3() {
    let Some((_b, _h, _d, table)) = open(&["basic1", "basic1.pltA"]) else {
        return;
    };
    // basic1 ships several `Node Labels…` / `Element Labels…` TI_PARAM
    // entries inline in the main `.A` directory. The table must index
    // them alongside MILI_PARAM / APPLICATION_PARAM entries.
    let mut ti_pattern_count = 0;
    for n in table.names() {
        if n.starts_with("Node Labels") || n.starts_with("Element Labels") {
            ti_pattern_count += 1;
        }
    }
    assert!(
        ti_pattern_count > 0,
        "expected inline TI_PARAM entries in basic1's main .A directory"
    );
}
