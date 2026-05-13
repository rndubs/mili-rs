//! Pure-Rust reader and writer for LLNL MDG mili databases.
//!
//! See `planning/mili-rs/plan.md` and `planning/shared/format.md` in the
//! repository root for the design and the on-disk format reference.
//!
//! Step 0 ships only the workspace skeleton and [`MiliError`]; the parser,
//! query path, and write path arrive in later steps per the plan.

mod error;

pub use error::{MiliError, Result};
