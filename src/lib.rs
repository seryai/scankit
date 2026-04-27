//! # scankit — walk + watch + filter directory trees.
//!
//! `scankit` is the shared scanner that Tauri / Iced / native
//! desktop apps reach for when they need to enumerate user files.
//! Its job is small but easy to get wrong:
//!
//! 1. Walk a directory tree (`walkdir` under the hood).
//! 2. Skip what the user said to skip — `.DS_Store`, `node_modules`,
//!    `.git`, `*.log`, anything matching the configured glob set.
//! 3. Drop oversized files before you ever read them — a rogue
//!    50 GB sqlite database shouldn't take your indexer offline.
//! 4. (Future, behind `watch` feature) keep watching the tree and
//!    emit change events as files are added / modified / removed.
//!
//! What `scankit` deliberately does NOT do:
//!
//! - Parse files. Use [`mdkit`](https://crates.io/crates/mdkit) or
//!   bring your own. `scankit` hands you `ScanEntry`s and gets out
//!   of the way.
//! - Schema extraction, search indexing, embedding generation.
//!   Those are the layers that consume `scankit`'s output.
//! - PII redaction, secrets scanning. Privacy policy is the
//!   embedding application's concern.
//!
//! ## Quick start
//!
//! ```no_run
//! use scankit::{Scanner, ScanConfig};
//! use std::path::Path;
//!
//! let scanner = Scanner::new(
//!     ScanConfig::default()
//!         .max_file_size_bytes(50 * 1024 * 1024) // 50 MB cap
//!         .add_exclude("**/.git/**")?
//!         .add_exclude("**/node_modules/**")?
//!         .add_exclude("**/.DS_Store")?,
//! )?;
//!
//! for result in scanner.walk(Path::new("/Users/me/Documents")) {
//!     match result {
//!         Ok(entry) => println!("{}: {} bytes", entry.path.display(), entry.size_bytes),
//!         Err(e)    => eprintln!("scan error: {e}"),
//!     }
//! }
//! # Ok::<(), scankit::Error>(())
//! ```
//!
//! ## Why a separate crate
//!
//! Every "index files on the user's machine" project rebuilds the
//! same five hundred lines of walkdir-with-excludes-and-size-cap
//! glue, and every project gets it slightly wrong. `scankit` ships
//! it once, with the edge cases (symlink loops, permission denials,
//! mid-walk concurrent deletes) handled in one place.

#![doc(html_root_url = "https://docs.rs/scankit")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::path::PathBuf;
use std::time::SystemTime;

mod error;
pub use error::{Error, Result};

#[cfg(feature = "walk")]
mod walk;
#[cfg(feature = "walk")]
pub use walk::{ScanWalkIter, Scanner};

#[cfg(feature = "watch")]
mod watch;
#[cfg(feature = "watch")]
pub use watch::{ScanEvent, ScanStream};

// ---------------------------------------------------------------------------
// ScanEntry — the unit of output
// ---------------------------------------------------------------------------

/// One file produced by a successful walk. Directories are not
/// surfaced — `Scanner` recurses into them silently. Symlinks are
/// dereferenced when [`ScanConfig::follow_symlinks`] is true and
/// emitted as the target file; otherwise they're skipped.
///
/// `#[non_exhaustive]` so we can grow the struct (e.g. add inode /
/// content hash) in minor versions without breaking external
/// struct-literal construction.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ScanEntry {
    /// Absolute path to the file, as walked. May contain
    /// non-UTF-8 components on platforms that allow them.
    pub path: PathBuf,
    /// File size in bytes at the time of stat.
    pub size_bytes: u64,
    /// Last-modified time per the filesystem. May be `None` on
    /// filesystems that don't track it (or on platforms that don't
    /// expose it through `std::fs::Metadata`).
    pub modified: Option<SystemTime>,
    /// File extension (lowercase, no leading dot). Empty when the
    /// file has no extension, or when the extension contains
    /// non-UTF-8 bytes that we can't normalise. Pre-computed here
    /// because callers almost always dispatch by extension and it's
    /// cheaper to compute it once during the walk than per-file
    /// downstream.
    pub extension: String,
}

// ---------------------------------------------------------------------------
// ScanConfig — the policy
// ---------------------------------------------------------------------------

/// Configuration for a [`Scanner`]. Construct via [`ScanConfig::default`]
/// then layer on options with the `with_*` / `add_*` builder methods,
/// or build from struct literal during the same crate.
///
/// `#[non_exhaustive]` — same forward-compat reasoning as
/// [`ScanEntry`].
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ScanConfig {
    /// Skip files whose size exceeds this limit, in bytes. `None`
    /// means no cap (the default — a `Scanner::walk` call will
    /// happily yield a 50 GB file if the caller asked for it).
    pub max_file_size_bytes: Option<u64>,
    /// Glob patterns matched against the full path; matching files
    /// (and directories — globs match `**/.git/**` against e.g.
    /// `/Users/me/proj/.git/HEAD` and exclude the whole subtree)
    /// are silently skipped. Empty by default; build the set with
    /// [`ScanConfig::add_exclude`].
    ///
    /// We hold the source `Glob`s rather than a built `GlobSet`
    /// because `GlobSet` is immutable post-build and doesn't expose
    /// its members for round-tripping. `Scanner::new` builds the
    /// `GlobSet` once at construction time from this list.
    #[cfg(feature = "walk")]
    pub excludes: Vec<globset::Glob>,
    /// When true, follow symlinks as if they were real files. When
    /// false (default), symlinks are skipped. Following symlinks
    /// risks both infinite loops (handled by `walkdir`) and crossing
    /// out of the tree the user thought they were scanning.
    pub follow_symlinks: bool,
}

impl ScanConfig {
    /// Set the per-file size cap. Files larger than `bytes` are
    /// silently skipped during the walk.
    #[must_use]
    pub fn max_file_size_bytes(mut self, bytes: u64) -> Self {
        self.max_file_size_bytes = Some(bytes);
        self
    }

    /// Toggle symlink following. Off by default.
    #[must_use]
    pub fn follow_symlinks(mut self, follow: bool) -> Self {
        self.follow_symlinks = follow;
        self
    }

    /// Add a glob pattern to the exclude set. Patterns are matched
    /// against the full absolute path; use `**` to match any path
    /// segment.
    ///
    /// Examples:
    /// - `**/.git/**` — exclude every `.git` directory
    /// - `**/*.log` — exclude every `.log` file
    /// - `**/node_modules/**` — exclude every `node_modules` tree
    ///
    /// Returns `Self` so calls can chain. Returns `Err` when the
    /// pattern is malformed (typically a stray `\` or unbalanced
    /// `[...]`).
    #[cfg(feature = "walk")]
    pub fn add_exclude(mut self, pattern: &str) -> Result<Self> {
        let glob = globset::Glob::new(pattern)
            .map_err(|e| Error::InvalidExclude(format!("`{pattern}`: {e}")))?;
        self.excludes.push(glob);
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_no_cap_and_no_excludes() {
        let cfg = ScanConfig::default();
        assert!(cfg.max_file_size_bytes.is_none());
        assert!(!cfg.follow_symlinks);
    }

    #[test]
    fn size_cap_builder_chains() {
        let cfg = ScanConfig::default().max_file_size_bytes(1024);
        assert_eq!(cfg.max_file_size_bytes, Some(1024));
    }

    #[test]
    fn follow_symlinks_builder_chains() {
        let cfg = ScanConfig::default().follow_symlinks(true);
        assert!(cfg.follow_symlinks);
    }

    #[cfg(feature = "walk")]
    #[test]
    fn add_exclude_accepts_valid_glob() {
        let cfg = ScanConfig::default()
            .add_exclude("**/.git/**")
            .expect("valid glob should accept");
        assert_eq!(cfg.excludes.len(), 1);
    }

    #[cfg(feature = "walk")]
    #[test]
    fn add_exclude_rejects_malformed_glob() {
        let result = ScanConfig::default().add_exclude("[unbalanced");
        assert!(matches!(result, Err(Error::InvalidExclude(_))));
    }
}
