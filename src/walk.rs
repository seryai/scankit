//! Directory walking: recursive enumeration with exclude-glob and
//! size-cap filters.
//!
//! Backed by [`walkdir`](https://crates.io/crates/walkdir) for the
//! actual tree traversal (loop detection, permission handling,
//! `Send` iterators) and [`globset`](https://crates.io/crates/globset)
//! for the exclude matching (compiled glob set, single-pass).

use crate::{Error, Result, ScanConfig, ScanEntry};
use globset::GlobSet;
use std::path::Path;
use walkdir::WalkDir;

/// A configured scanner. Cheap to construct (the `GlobSet` build
/// is the only non-trivial work, ~µs for typical exclude lists).
/// `Send + Sync` — share a single `Scanner` across threads.
pub struct Scanner {
    config: ScanConfig,
    excludes: GlobSet,
}

impl Scanner {
    /// Construct from a [`ScanConfig`]. Builds the internal
    /// `GlobSet` once; subsequent walks reuse it.
    ///
    /// Returns `Err(Error::InvalidExclude)` if the config's exclude
    /// list contains a pattern that compiles individually but
    /// triggers an internal `GlobSet` build error (vanishingly rare
    /// in practice — `globset` only fails to build on invariant
    /// violations that should have been caught by `Glob::new`).
    pub fn new(config: ScanConfig) -> Result<Self> {
        let mut builder = globset::GlobSetBuilder::new();
        for glob in &config.excludes {
            builder.add(glob.clone());
        }
        let excludes = builder
            .build()
            .map_err(|e| Error::InvalidExclude(e.to_string()))?;
        Ok(Self { config, excludes })
    }

    /// One-shot walk of `root`. Returns an iterator yielding
    /// `Result<ScanEntry>`:
    ///
    /// - `Ok(entry)` — a regular file that passed all filters.
    /// - `Err(Error::Walk(_))` — `walkdir` failed mid-iteration
    ///   (typically permission denial on a subdirectory). The
    ///   iterator continues after the error; callers can choose
    ///   to log-and-continue or short-circuit on first error.
    ///
    /// Filtered-out entries (excludes, oversized files,
    /// directories, symlinks when not following) are silently
    /// dropped from the iterator — they don't surface as errors.
    pub fn walk<P: AsRef<Path>>(&self, root: P) -> ScanWalkIter<'_> {
        let walker = WalkDir::new(root.as_ref())
            .follow_links(self.config.follow_symlinks)
            .into_iter();
        ScanWalkIter {
            scanner: self,
            inner: walker,
        }
    }

    /// Return the active config. Useful for diagnostic logging
    /// after construction.
    #[must_use]
    pub fn config(&self) -> &ScanConfig {
        &self.config
    }
}

/// Iterator returned by [`Scanner::walk`]. Yields one `Result`
/// per file emitted; lazy under the hood.
pub struct ScanWalkIter<'a> {
    scanner: &'a Scanner,
    inner: walkdir::IntoIter,
}

impl Iterator for ScanWalkIter<'_> {
    type Item = Result<ScanEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let dir_entry = match self.inner.next()? {
                Ok(de) => de,
                Err(e) => return Some(Err(Error::from(e))),
            };

            // Skip directories — only files surface in the iterator.
            // walkdir yields directories too; we filter them here
            // rather than via `min_depth(1).max_depth(...)` because
            // exclude globs need to apply to directory paths so
            // matching subtrees don't get descended into.
            if dir_entry.file_type().is_dir() {
                if self.is_excluded(dir_entry.path()) {
                    self.inner.skip_current_dir();
                }
                continue;
            }

            // Symlinks — walkdir already followed them when
            // `follow_links(true)` was set, in which case
            // `is_dir()` / `is_file()` reflect the target. When
            // false, symlinks come through as their own type and
            // we skip them.
            if dir_entry.file_type().is_symlink() {
                continue;
            }

            // Exclude matching against the full path.
            if self.is_excluded(dir_entry.path()) {
                continue;
            }

            // Stat for size + modified time. A failure here is
            // typically a TOCTOU race (file vanished between
            // walkdir's stat and ours) — treat as "skip silently"
            // rather than yielding an error.
            let Ok(metadata) = dir_entry.metadata() else {
                continue;
            };

            let size_bytes = metadata.len();
            if let Some(cap) = self.scanner.config.max_file_size_bytes {
                if size_bytes > cap {
                    continue;
                }
            }

            let modified = metadata.modified().ok();
            let extension = dir_entry
                .path()
                .extension()
                .and_then(|os| os.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();

            return Some(Ok(ScanEntry {
                path: dir_entry.into_path(),
                size_bytes,
                modified,
                extension,
            }));
        }
    }
}

impl ScanWalkIter<'_> {
    fn is_excluded(&self, path: &Path) -> bool {
        // GlobSet matches against any path representation; we
        // pass the platform-native path. Patterns like
        // `**/.git/**` portably match both `/Users/me/.git/HEAD`
        // and `C:\Users\me\.git\HEAD`.
        self.scanner.excludes.is_match(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "hello").unwrap();
        fs::write(root.join("b.log"), "noise").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        fs::create_dir_all(root.join("nested/deep")).unwrap();
        fs::write(root.join("nested/c.md"), "# title").unwrap();
        fs::write(root.join("nested/deep/d.csv"), "a,b\n1,2").unwrap();
        dir
    }

    #[test]
    fn walks_all_files_by_default() {
        let dir = make_tree();
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let entries: Vec<_> = scanner.walk(dir.path()).filter_map(Result::ok).collect();
        // 5 files: a.txt, b.log, .git/HEAD, nested/c.md, nested/deep/d.csv
        assert_eq!(entries.len(), 5, "got {entries:?}");
    }

    #[test]
    fn excludes_match_files() {
        let dir = make_tree();
        let scanner = Scanner::new(ScanConfig::default().add_exclude("**/*.log").unwrap()).unwrap();
        let extensions: Vec<_> = scanner
            .walk(dir.path())
            .filter_map(Result::ok)
            .map(|e| e.extension)
            .collect();
        assert!(!extensions.contains(&"log".to_string()));
        assert_eq!(extensions.len(), 4);
    }

    #[test]
    fn excludes_match_directories_and_skip_subtree() {
        let dir = make_tree();
        let scanner =
            Scanner::new(ScanConfig::default().add_exclude("**/.git/**").unwrap()).unwrap();
        let paths: Vec<_> = scanner
            .walk(dir.path())
            .filter_map(Result::ok)
            .map(|e| e.path)
            .collect();
        assert!(
            !paths.iter().any(|p| p.to_string_lossy().contains("/.git/")),
            "got {paths:?}"
        );
    }

    #[test]
    fn size_cap_filters_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("small.txt"), "tiny").unwrap();
        fs::write(dir.path().join("big.txt"), vec![0u8; 200]).unwrap();
        let scanner = Scanner::new(ScanConfig::default().max_file_size_bytes(100)).unwrap();
        let entries: Vec<_> = scanner.walk(dir.path()).filter_map(Result::ok).collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].path.ends_with("small.txt"));
    }

    #[test]
    fn extension_is_lowercased_and_dotless() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("upper.PDF"), b"").unwrap();
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let entries: Vec<_> = scanner.walk(dir.path()).filter_map(Result::ok).collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].extension, "pdf");
    }

    #[test]
    fn extensionless_files_get_empty_string() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README"), b"readme").unwrap();
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let entries: Vec<_> = scanner.walk(dir.path()).filter_map(Result::ok).collect();
        assert_eq!(entries[0].extension, "");
    }

    #[test]
    fn missing_root_returns_error_via_iter() {
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let mut iter = scanner.walk("/this/path/cannot/exist/9f3a2b1c");
        // walkdir surfaces the missing-root failure as the first
        // iter element rather than a top-level error — match the
        // shape so we don't panic on bad input.
        let first = iter.next();
        assert!(matches!(first, Some(Err(_))));
    }
}
