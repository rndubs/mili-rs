//! Pure-Rust reader for LLNL MDG mili databases.
//!
//! Design + on-disk format reference: `planning/mili-rs/plan.md`,
//! `planning/shared/format.md`, and `planning/shared/entry-payloads.md`
//! in the repository root.
//!
//! ## Public surface
//!
//! The crate-root re-exports below are the supported API. Top-level
//! `Database::open` returns a memory-mapped reader; `Database::query`
//! materialises one or more svars into a flat `StateValues` buffer.
//!
//! Internal types (`Directory`, `DirEntry`, raw `Srec`/`Subrecord`,
//! the `*Table` indexing structs, `decode_*` free functions,
//! `Lumps` + `derive_lumps`) are marked `#[doc(hidden)]` — they're
//! re-exported so existing in-tree integration tests still resolve
//! through `use mili_rs::...`, but consumers (in particular the
//! upcoming `mili-py` crate) should prefer the high-level
//! `Database::*` accessors over poking at directory / srec internals.

#![allow(clippy::result_large_err)] // MiliError carries enough context that boxing isn't worth it.

pub(crate) mod buffer;
mod directory;
pub(crate) mod endian;
mod error;
pub mod family;
pub mod family_set;
pub mod header;
mod mesh;
mod param;
mod query;
mod srec;
mod state;
mod svar;
mod ti;

#[doc(hidden)]
pub use directory::{ByteRange, DirEntry, DirEntryType, Directory, NamePool};
pub use error::{MiliError, Result};
pub use family::{Database, QueryArgs};
pub use family_set::{DatabaseSet, SetQueryResult};
pub use header::{Endianness, Header, PartitionScheme, PrecisionLimit};
#[doc(hidden)]
pub use mesh::{decode_elem_conns, decode_nodes, MeshTable};
pub use mesh::{Connectivity, MaterialId, Mesh, MeshId, Nodes, ObjectClass, Superclass};
#[doc(hidden)]
pub use param::ParamTable;
pub use param::{AggType, ArrayParam, DataType, ParamValue, ScalarValue};
pub use query::{QueryResult, StateValues};
pub use srec::Organization;
#[doc(hidden)]
pub use srec::{derive_lumps, Lumps, Srec, SrecTable, Subrecord};
#[doc(hidden)]
pub use state::{parse_inline, parse_tfile, tfile_path, StateMapSource, StateMeta};
#[doc(hidden)]
pub use svar::SvarTable;
pub use svar::{NumType, Svar, SvarAgg};
