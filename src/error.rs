//! Crate-wide error type for the library surface.
//!
//! Storage and state operations return [`Result`] so that callers (and the
//! public API) never depend directly on `rusqlite`. Binaries layer `anyhow` on
//! top of this for top-level reporting.

use thiserror::Error;

/// Errors produced by the perbot library.
#[derive(Debug, Error)]
pub enum Error {
    /// A SQLite operation failed (query, execution, or row conversion).
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Convenience alias for results returned by the library.
pub type Result<T> = std::result::Result<T, Error>;
