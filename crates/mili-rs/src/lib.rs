//! Pure-Rust reader and writer for LLNL MDG mili databases.
//!
//! See `planning/mili-rs/plan.md` and `planning/shared/format.md` in the
//! repository root for the design and the on-disk format reference.
//!
//! Step 0 ships only the workspace skeleton and [`MiliError`]; the parser,
//! query path, and write path arrive in later steps per the plan.

pub mod directory;
mod error;
pub mod family;
pub mod header;
pub mod mesh;
pub mod param;
pub mod srec;
pub mod state;
pub mod svar;
pub mod ti;

pub use directory::{ByteRange, DirEntry, DirEntryType, Directory, NamePool};
pub use error::{MiliError, Result};
pub use family::Database;
pub use header::{Endianness, Header, PartitionScheme, PrecisionLimit};
pub use mesh::{
    decode_elem_conns, decode_nodes, Connectivity, MaterialId, Mesh, MeshId, MeshTable, Nodes,
    ObjectClass, Superclass,
};
pub use param::{AggType, ArrayParam, DataType, ParamTable, ParamValue, ScalarValue};
pub use srec::{derive_lumps, Lumps, Organization, Srec, SrecTable, Subrecord};
pub use state::{StateMapSource, StateMeta};
pub use svar::{NumType, Svar, SvarAgg, SvarTable};
