//! `Database` — the opened-mili-family handle.
//!
//! `Database::open(path)` is the user's entry point. Given a path to a
//! family's `.A` file (e.g. `data/serial/basic1/basic1.pltA`) it:
//!
//! 1. Memory-maps the `.A` file so byte ranges from directory entries
//!    remain valid for the database's lifetime.
//! 2. Parses the [`Header`] and [`Directory`].
//! 3. Builds the [`ParamTable`] index over the directory's
//!    `MILI_PARAM` / `APPLICATION_PARAM` / `TI_PARAM` entries.
//! 4. Loads the state map — inline from the `.A` trailer or from the
//!    sibling `<root>T` tfile, whichever the layout dictates
//!    (`StateMapSource::pick`).
//!
//! Lazy state-file mapping is not yet implemented; that lands when
//! the read path (`query.rs`) needs it. The family root is stashed so
//! state files (`<root>00`, `<root>01`, …) can be resolved by index.

use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::directory::Directory;
use crate::error::{MiliError, Result};
use crate::header::Header;
use crate::param::{ParamTable, ParamValue, ScalarValue};
use crate::state::{self, StateMapSource, StateMeta};

/// An opened mili family — the read-side handle through which all
/// parsed metadata and state byte ranges are reachable.
pub struct Database {
    a_path: PathBuf,
    a_mmap: Mmap,
    header: Header,
    directory: Directory,
    params: ParamTable,
    states: Vec<StateMeta>,
}

impl Database {
    /// Open a mili family given the path to its `.A` file.
    ///
    /// Sibling state files (`<root>00`, `<root>01`, …) are not mapped
    /// at open time. The tfile (`<root>T`) is read once if the
    /// directory indicates the state map lives there; otherwise it is
    /// not opened.
    pub fn open(a_path: impl AsRef<Path>) -> Result<Self> {
        let a_path = a_path.as_ref().to_path_buf();
        let a_mmap = mmap_read_only(&a_path)?;

        let header = Header::parse(&a_mmap)?;
        let directory = Directory::parse(&a_mmap, &header)?;
        let params = ParamTable::build(&directory);

        let states = match StateMapSource::pick(&header, &directory) {
            StateMapSource::InlineA(range) => state::parse_inline(&a_mmap, range, &header)?,
            StateMapSource::ExternalTfile => {
                let path = state::tfile_path(&a_path).ok_or(MiliError::MalformedDirectory(
                    "cannot derive tfile path from .A path",
                ))?;
                let bytes = std::fs::read(&path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        MiliError::MalformedDirectory(
                            "directory expects tfile but no <root>T sibling found",
                        )
                    } else {
                        e.into()
                    }
                })?;
                state::parse_tfile(&bytes, &header)?
            }
        };

        Ok(Self {
            a_path,
            a_mmap,
            header,
            directory,
            params,
            states,
        })
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn params(&self) -> &ParamTable {
        &self.params
    }

    /// Per-state metadata in directory order.
    pub fn states(&self) -> &[StateMeta] {
        &self.states
    }

    /// Bytes of the main `.A` file. Byte ranges from
    /// [`crate::directory::DirEntry`] index into this slice.
    pub fn a_bytes(&self) -> &[u8] {
        &self.a_mmap
    }

    /// Path to the `.A` file this database was opened from.
    pub fn a_path(&self) -> &Path {
        &self.a_path
    }

    /// Decode a named scalar / string / array param from the main
    /// directory. Returns `None` if no such named entry exists.
    pub fn param(&self, name: &str) -> Result<Option<ParamValue<'_>>> {
        let Some(idx) = self.params.get(name) else {
            return Ok(None);
        };
        let entry = &self.directory.entries[idx];
        Ok(Some(ParamValue::decode(&self.a_mmap, entry, self.header)?))
    }

    /// Read the `"mesh dimensions"` scalar (`mesh_u.c:296-299, 3900`).
    /// Errors if the param is absent or not an i32 scalar.
    pub fn mesh_dimensions(&self) -> Result<i32> {
        match self.param("mesh dimensions")? {
            Some(ParamValue::Scalar(ScalarValue::I32(n))) => Ok(n),
            Some(_) => Err(MiliError::MalformedDirectory(
                "'mesh dimensions' is not an i32 scalar",
            )),
            None => Err(MiliError::MalformedDirectory(
                "'mesh dimensions' param not found",
            )),
        }
    }

    /// Times in directory order. Convenience over `states().iter().map(|m| m.time)`.
    pub fn times(&self) -> Vec<f32> {
        self.states.iter().map(|m| m.time).collect()
    }

    pub fn state_count(&self) -> usize {
        self.states.len()
    }
}

fn mmap_read_only(path: &Path) -> Result<Mmap> {
    let file = File::open(path)?;
    // SAFETY: the mmap2 contract requires that the underlying file
    // not be mutated externally for the lifetime of the mapping. We
    // hold an open `File` handle through Database lifetime and never
    // expose write access, but the user could in principle modify
    // the file out-of-band. That mirrors the C reader's posture —
    // mili databases are not concurrent-write safe.
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(mmap)
}
