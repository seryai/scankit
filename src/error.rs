//! Error and Result types for scankit.
//!
//! All public APIs that can fail return [`Result<T>`]. Errors are
//! categorised broadly so callers can map them to user-facing
//! messages without pattern-matching on opaque strings.

use std::io;
use thiserror::Error;

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can arise during scanning.
///
/// Marked `#[non_exhaustive]` so future minor versions can add new
/// variants (e.g. a dedicated `WatchTimeout` once the `watch`
/// feature lands) without breaking downstream `match` blocks.
/// Callers that pattern-match should always include a wildcard arm.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// A glob pattern passed to [`ScanConfig::add_exclude`](crate::ScanConfig::add_exclude)
    /// was malformed (typically a stray `\` or unbalanced `[...]`).
    #[error("invalid exclude pattern: {0}")]
    InvalidExclude(String),

    /// Filesystem failure while reading a directory entry's metadata.
    /// The walker logs and skips these by default — they only
    /// surface in `Result` form when the caller asks for raw error
    /// stream via the iterator's `Err` branch.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// `walkdir`-internal failure (loop detection, permission
    /// denial, etc.). Wraps the underlying error verbatim.
    #[cfg(feature = "walk")]
    #[error("walk error: {0}")]
    Walk(#[from] walkdir::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_convert_via_from() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "missing");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn invalid_exclude_renders_message() {
        let err = Error::InvalidExclude("`[unbalanced`: ...".into());
        assert!(err.to_string().contains("invalid exclude"));
    }
}
