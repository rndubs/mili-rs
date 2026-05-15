//! `PyMiliDatabase` — a thin wrapper over `mili_rs::Database` *or*
//! `mili_rs::DatabaseSet`, chosen at open time by fragment count (the
//! pinned FFI contract: no `LoopWrapper` / `ServerWrapper` port; the
//! fan-out/merge job lives in `DatabaseSet`).
//!
//! M1 binds the read-only metadata surface only. Returns are small
//! Python lists / ints / dicts; bulk arrays (`nodes`, `connectivity`,
//! `query`) and the `IntoPyArray` zero-copy path land in M2/M3.

use std::collections::HashMap;
use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use mili_rs::{Database, DatabaseSet, MeshId};

use crate::errors::to_pyerr;

// Boxed: `Database` (mmap + tables) and `DatabaseSet` (a `Vec` of
// them) differ enormously in size; boxing keeps the enum small.
enum Backend {
    Single(Box<Database>),
    Set(Box<DatabaseSet>),
}

#[pyclass(name = "PyMiliDatabase", module = "milox._native")]
pub struct PyMiliDatabase {
    backend: Backend,
    /// Canonical mesh id (smallest present; the corpus is single-mesh).
    /// Upstream's metadata surface is mesh-global; we resolve the one
    /// mesh once at open.
    mesh: MeshId,
}

fn canonical_mesh(db: &Database) -> MeshId {
    db.meshes()
        .meshes()
        .map(|m| m.id)
        .min()
        .unwrap_or(MeshId(0))
}

#[pymethods]
impl PyMiliDatabase {
    /// Single-fragment open. The Python shim resolves the `.A` file
    /// path (filename-root parsing stays in Python).
    #[staticmethod]
    fn open_single(py: Python<'_>, a_path: PathBuf) -> PyResult<Self> {
        let db = py
            .allow_threads(|| Database::open(&a_path))
            .map_err(|e| to_pyerr(&e))?;
        let mesh = canonical_mesh(&db);
        Ok(Self {
            backend: Backend::Single(Box::new(db)),
            mesh,
        })
    }

    /// Multi-fragment open. `base` is the family base path;
    /// `DatabaseSet::open` re-discovers fragments and fans out (rayon —
    /// hence `allow_threads`).
    #[staticmethod]
    fn open_set(py: Python<'_>, base: PathBuf) -> PyResult<Self> {
        let set = py
            .allow_threads(|| DatabaseSet::open(&base))
            .map_err(|e| to_pyerr(&e))?;
        let mesh = set.fragment(0).map_or(MeshId(0), canonical_mesh);
        Ok(Self {
            backend: Backend::Set(Box::new(set)),
            mesh,
        })
    }

    /// Simulation time per state. Upstream returns `np.float64`; we
    /// widen `f32` → `f64` here so a direct numeric compare matches.
    fn times(&self) -> Vec<f64> {
        let t = match &self.backend {
            Backend::Single(db) => db.times(),
            Backend::Set(s) => s.times(),
        };
        t.into_iter().map(f64::from).collect()
    }

    fn state_count(&self) -> usize {
        match &self.backend {
            Backend::Single(db) => db.state_count(),
            Backend::Set(s) => s.state_count(),
        }
    }

    fn mesh_dimensions(&self) -> PyResult<i32> {
        match &self.backend {
            Backend::Single(db) => db.mesh_dimensions(),
            Backend::Set(s) => s.mesh_dimensions(),
        }
        .map_err(|e| to_pyerr(&e))
    }

    fn class_names(&self) -> Vec<String> {
        match &self.backend {
            Backend::Single(db) => db.class_names(self.mesh),
            Backend::Set(s) => s.class_names(self.mesh),
        }
    }

    /// `{class_name: [labels]}` over every class that declares labels
    /// (upstream's no-arg `labels()` returns the full dict).
    fn labels<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let mesh = self.mesh;
        let collected: Vec<(String, Vec<i32>)> = py.allow_threads(|| {
            let names = match &self.backend {
                Backend::Single(db) => db.class_names(mesh),
                Backend::Set(s) => s.class_names(mesh),
            };
            let mut out = Vec::new();
            for name in names {
                let labels = match &self.backend {
                    Backend::Single(db) => db.labels(mesh, &name),
                    Backend::Set(s) => s.labels(mesh, &name),
                };
                if let Ok(Some(v)) = labels {
                    out.push((name, v));
                }
            }
            out
        });
        let d = PyDict::new_bound(py);
        for (k, v) in collected {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// Per-state map. Upstream `StateMap` carries `file_number`,
    /// `file_offset`, `time`; we expose the comparable fields as a list
    /// of dicts. (`state_map_id` has no direct `mili-rs` analogue and
    /// is not part of the decision-4 rank-0 parity contract.)
    fn state_maps<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let smaps = match &self.backend {
            Backend::Single(db) => db.states(),
            Backend::Set(s) => s.state_maps(),
        };
        let mut out = Vec::with_capacity(smaps.len());
        for sm in smaps {
            let d = PyDict::new_bound(py);
            d.set_item("file_number", sm.file)?;
            d.set_item("file_offset", sm.offset)?;
            d.set_item("time", f64::from(sm.time))?;
            out.push(d);
        }
        Ok(out)
    }

    fn materials<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.materials(),
                Backend::Set(s) => s.materials(),
            })
            .map_err(|e| to_pyerr(&e))?;
        map_to_pydict(py, m)
    }

    fn material_numbers(&self, py: Python<'_>) -> PyResult<Vec<i32>> {
        py.allow_threads(|| match &self.backend {
            Backend::Single(db) => db.material_numbers(),
            Backend::Set(s) => s.material_numbers(),
        })
        .map_err(|e| to_pyerr(&e))
    }

    fn element_sets<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.element_sets(),
                Backend::Set(s) => s.element_sets(),
            })
            .map_err(|e| to_pyerr(&e))?;
        map_to_pydict(py, m)
    }

    /// Per-material integration points. Upstream keys are strings
    /// (`element-set name[-1:]`); `mili-rs` keys by parsed material id,
    /// so we stringify the id. Matches upstream for single-digit set
    /// names (the corpus); see status.md "Element-set name → material
    /// id parse rule".
    fn integration_points<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.integration_points(),
                Backend::Set(s) => s.integration_points(),
            })
            .map_err(|e| to_pyerr(&e))?;
        let d = PyDict::new_bound(py);
        for (mat, ips) in m {
            d.set_item(mat.0.to_string(), ips)?;
        }
        Ok(d)
    }
}

fn map_to_pydict(py: Python<'_>, m: HashMap<String, Vec<i32>>) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new_bound(py);
    for (k, v) in m {
        d.set_item(k, v)?;
    }
    Ok(d)
}
