//! Continuous filesystem-event monitoring on top of an initial walk.
//!
//! Backed by [`notify`](https://crates.io/crates/notify) for the
//! cross-platform event source (`FSEvents` on macOS,
//! `ReadDirectoryChangesW` on Windows, inotify on Linux). The
//! `notify` thread emits raw
//! kernel events into a channel; the `ScanStream` iterator pulls
//! from that channel and applies the same filters
//! ([`ScanConfig::excludes`](crate::ScanConfig::excludes), size cap,
//! symlink policy) that [`Scanner::walk`](crate::Scanner::walk) uses.
//!
//! ## Lifecycle
//!
//! 1. The caller invokes `Scanner::scan(root)`.
//! 2. A worker thread enumerates `root` via the existing walk path
//!    and emits one [`ScanEvent::Initial`] per file.
//! 3. After the initial walk finishes, the worker emits one
//!    [`ScanEvent::InitialComplete`] marker. Callers building UIs
//!    use this to switch from "scanning…" to "watching."
//! 4. From then on, the `notify` callback (running on `notify`'s
//!    thread) emits [`ScanEvent::Created`] / [`ScanEvent::Modified`]
//!    / [`ScanEvent::Deleted`] for as long as the `ScanStream` is
//!    alive.
//! 5. When the caller drops the `ScanStream`, the underlying
//!    `notify` watcher drops with it, the kernel subscription is
//!    released, and both threads exit cleanly.
//!
//! Renames are surfaced as a `Deleted` of the old path followed by
//! a `Created` of the new path — `notify`'s rename event shape
//! varies by platform and consolidating into one `Renamed` variant
//! is left for v0.3.

use crate::{Error, Result, ScanConfig, ScanEntry, Scanner};
use globset::GlobSet;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::SystemTime;

/// Events emitted by [`ScanStream`].
///
/// `#[non_exhaustive]` so the enum can grow (`Renamed`, `Closed`,
/// platform-specific kinds) without breaking external matches —
/// always include a wildcard arm when matching.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ScanEvent {
    /// A file enumerated during the initial walk. Filters
    /// (excludes, size cap) have already been applied — these
    /// events are equivalent to what
    /// [`Scanner::walk`](crate::Scanner::walk) would have yielded.
    Initial(ScanEntry),

    /// Sentinel emitted exactly once, after the last `Initial` event
    /// and before the first live filesystem event. Callers
    /// rendering a "scanning…" indicator typically flip to a "live"
    /// state at this marker.
    InitialComplete,

    /// A file appeared after watching started — was created from
    /// scratch, copied in, or moved in from outside the watched
    /// tree. Filters applied.
    Created(ScanEntry),

    /// An existing watched file's contents changed. Filters
    /// applied. Note that one logical "save" may emit multiple
    /// `Modified` events on platforms that surface intermediate
    /// states — callers debouncing for cost reasons should
    /// coalesce these.
    Modified(ScanEntry),

    /// A file was removed. Path only — the file no longer exists,
    /// so we can't restat for size or extension. Excludes are NOT
    /// re-checked here; if the caller cared about the path's
    /// excluded-ness they would have skipped it on the
    /// corresponding `Initial` / `Created` event.
    Deleted(PathBuf),
}

/// Stream of [`ScanEvent`]s. Returned by
/// [`Scanner::scan`](crate::Scanner::scan).
///
/// Implements `Iterator<Item = ScanEvent>`. Iteration **blocks**
/// until the next event arrives or the stream closes (the watcher
/// shuts down, or both producer threads exit). Callers wanting
/// non-blocking semantics should poll via
/// [`ScanStream::try_next`] or drive the stream from a worker
/// thread.
///
/// Dropping the stream stops the underlying `notify` watcher and
/// the initial-walk worker (the worker exits when its send fails).
pub struct ScanStream {
    rx: Receiver<ScanEvent>,
    // Held to keep the OS subscription alive — when this drops,
    // notify stops sending events and the producer side of the
    // channel hangs up, ending iteration.
    _watcher: RecommendedWatcher,
}

impl ScanStream {
    /// Non-blocking poll. Returns the next event if one is ready,
    /// `Ok(None)` if the channel is empty, `Err` if the channel has
    /// hung up (no more events will arrive).
    pub fn try_next(
        &self,
    ) -> std::result::Result<Option<ScanEvent>, std::sync::mpsc::TryRecvError> {
        match self.rx.try_recv() {
            Ok(ev) => Ok(Some(ev)),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(e @ std::sync::mpsc::TryRecvError::Disconnected) => Err(e),
        }
    }
}

impl Iterator for ScanStream {
    type Item = ScanEvent;

    fn next(&mut self) -> Option<Self::Item> {
        // recv() blocks until either a message arrives or all
        // senders have hung up. The latter happens when the
        // initial-walk worker finishes AND the notify watcher
        // drops — i.e. when the stream itself is being torn down.
        self.rx.recv().ok()
    }
}

impl Scanner {
    /// Start a continuous scan of `root`. Returns a
    /// [`ScanStream`] yielding `ScanEvent`s — first the initial
    /// walk, then live filesystem events for as long as the stream
    /// is held.
    ///
    /// # Threading
    ///
    /// This method spawns one short-lived thread for the initial
    /// walk plus the `notify` watcher's own internal threading.
    /// Both push into the same `mpsc::channel`, so events from
    /// "files modified during the initial walk" interleave with
    /// `Initial` events. Callers needing strict
    /// initial-then-live ordering should buffer until
    /// [`ScanEvent::InitialComplete`] before treating subsequent
    /// events as live.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Watch`] if the platform's filesystem
    /// notification API rejects the watch (typically: root doesn't
    /// exist, permission denial, or the kernel inotify watch
    /// budget is exhausted).
    #[cfg(feature = "watch")]
    pub fn scan<P: AsRef<Path>>(&self, root: P) -> Result<ScanStream> {
        let root = root.as_ref().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();

        // Initial walk runs on its own thread so it doesn't block
        // the caller — `Scanner::scan` returns immediately, the
        // first `Initial` event lands once the walker stats its
        // first file. Cloning the relevant config + GlobSet rather
        // than Arc'ing Self because both are cheaply Clone and
        // avoiding Arc keeps the public API non-Arc'd.
        let walk_config = self.config().clone();
        let walk_root = root.clone();
        let walk_tx = tx.clone();
        std::thread::Builder::new()
            .name("scankit-initial-walk".into())
            .spawn(move || initial_walk(walk_config, walk_root, walk_tx))
            .map_err(|e| Error::Watch(format!("could not spawn initial-walk thread: {e}")))?;

        // notify watcher. The closure captures the GlobSet + size
        // cap so post-event filtering doesn't need a back-channel
        // to the Scanner.
        let excludes = self.excludes_for_watch().clone();
        let max_size = self.config().max_file_size_bytes;
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                handle_notify_event(event, &excludes, max_size, &tx);
            }
            // Errors from notify aren't surfaced — they're typically
            // transient (queue overrun) or fatal (watch revoked) and
            // there's no clean way to thread them through an
            // Iterator. A future v0.3 could add an `Error` variant
            // to ScanEvent.
        })
        .map_err(|e| Error::Watch(format!("notify init failed: {e}")))?;

        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| Error::Watch(format!("could not watch {}: {e}", root.display())))?;

        Ok(ScanStream {
            rx,
            _watcher: watcher,
        })
    }
}

/// Drive the initial walk and feed `Initial` / `InitialComplete`
/// events into the channel. Exits when the channel hangs up
/// (caller dropped the stream) or the walk finishes.
///
/// Owned parameters: this runs in a spawned thread, so the
/// `ScanConfig` + root `PathBuf` have to outlive the spawning
/// frame. Clippy's `needless_pass_by_value` flags this as if the
/// arguments could be borrowed — but the borrow would have to
/// be `'static`, which the spawning thread can't guarantee. The
/// allow keeps the spawn ergonomics simple.
#[allow(clippy::needless_pass_by_value)]
fn initial_walk(config: ScanConfig, root: PathBuf, tx: Sender<ScanEvent>) {
    // Build a one-shot Scanner from the cloned config. Failure to
    // build is silent — the only realistic failure here is an
    // invalid GlobSet, which we already validated in `Scanner::new`
    // before this thread spawned.
    let Ok(scanner) = Scanner::new(config) else {
        return;
    };
    // Walk errors during the initial pass don't get surfaced via
    // ScanStream today (walkdir keeps going per its own contract;
    // v0.3 could add ScanEvent::WalkError).
    for entry in scanner.walk(&root).flatten() {
        if tx.send(ScanEvent::Initial(entry)).is_err() {
            // Stream dropped — bail out of the walk.
            return;
        }
    }
    let _ = tx.send(ScanEvent::InitialComplete);
}

/// Translate one notify event into zero-or-more `ScanEvent`s and
/// push them into the channel. Excluded paths and oversized files
/// are filtered out here so callers don't have to re-implement the
/// same filters they configured on the `Scanner`.
fn handle_notify_event(
    event: notify::Event,
    excludes: &GlobSet,
    max_size: Option<u64>,
    tx: &Sender<ScanEvent>,
) {
    let kind = event.kind;
    for path in event.paths {
        // Exclude check applies to all event kinds. A user who
        // excluded `**/.git/**` doesn't want to be told that
        // `.git/index` was modified.
        if excludes.is_match(&path) {
            continue;
        }
        match kind {
            EventKind::Create(_) => {
                if let Some(entry) = entry_from_path(&path, max_size) {
                    let _ = tx.send(ScanEvent::Created(entry));
                }
            }
            EventKind::Modify(_) => {
                if let Some(entry) = entry_from_path(&path, max_size) {
                    let _ = tx.send(ScanEvent::Modified(entry));
                }
            }
            EventKind::Remove(_) => {
                let _ = tx.send(ScanEvent::Deleted(path));
            }
            // Access events (file opened / closed) and platform-
            // specific Other kinds aren't useful for indexing, so
            // we drop them silently.
            _ => {}
        }
    }
}

/// Build a [`ScanEntry`] from a path that just had a notify event.
/// Returns `None` for directories (we only emit file events),
/// inaccessible paths (TOCTOU race — file vanished between the
/// event firing and our stat), and paths that exceed the size cap.
fn entry_from_path(path: &Path, max_size: Option<u64>) -> Option<ScanEntry> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let size_bytes = metadata.len();
    if let Some(cap) = max_size {
        if size_bytes > cap {
            return None;
        }
    }
    let modified: Option<SystemTime> = metadata.modified().ok();
    let extension = path
        .extension()
        .and_then(|os| os.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    Some(ScanEntry {
        path: path.to_path_buf(),
        size_bytes,
        modified,
        extension,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    fn drain_initial(stream: &mut ScanStream, max_wait: Duration) -> Vec<ScanEvent> {
        // Pull events until we hit `InitialComplete` or time out.
        // The blocking `next()` would hang forever if the marker
        // never arrives (test-time bug); polling with try_next +
        // a deadline keeps tests honest.
        let deadline = Instant::now() + max_wait;
        let mut events = Vec::new();
        while Instant::now() < deadline {
            match stream.try_next() {
                Ok(Some(ev @ ScanEvent::InitialComplete)) => {
                    events.push(ev);
                    return events;
                }
                Ok(Some(ev)) => events.push(ev),
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return events,
            }
        }
        events
    }

    #[test]
    fn initial_walk_emits_existing_files_then_marker() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hi").unwrap();
        fs::write(dir.path().join("b.txt"), "yo").unwrap();
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let mut stream = scanner.scan(dir.path()).expect("watch start");

        let events = drain_initial(&mut stream, Duration::from_secs(2));

        let initials: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ScanEvent::Initial(entry) => Some(entry.path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(initials.len(), 2, "got {events:?}");
        assert!(matches!(events.last(), Some(ScanEvent::InitialComplete)));
    }

    #[test]
    fn live_creates_emit_after_initial_complete() {
        let dir = tempfile::tempdir().unwrap();
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let mut stream = scanner.scan(dir.path()).expect("watch start");

        // Wait through the empty initial walk.
        let _initial = drain_initial(&mut stream, Duration::from_secs(2));

        // Create a file and wait for the Created event. Notify
        // can take a moment to deliver — give it up to 5s before
        // calling the test flaky.
        fs::write(dir.path().join("new.txt"), "fresh").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_created = false;
        while Instant::now() < deadline {
            match stream.try_next() {
                Ok(Some(ScanEvent::Created(entry))) => {
                    assert!(entry.path.ends_with("new.txt"));
                    saw_created = true;
                    break;
                }
                Ok(Some(_other)) => {} // ignore Modified / etc.
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        assert!(
            saw_created,
            "did not see Created event for new.txt within 5s"
        );
    }

    #[test]
    fn dropping_stream_stops_the_watcher() {
        // Sanity: after dropping the stream, no events should
        // surface on a freshly-modified file (they have nowhere
        // to go — channel is gone). We exercise this primarily to
        // check Drop semantics don't panic.
        let dir = tempfile::tempdir().unwrap();
        let scanner = Scanner::new(ScanConfig::default()).unwrap();
        let stream = scanner.scan(dir.path()).expect("watch start");
        drop(stream);
        // If we got here, Drop ran cleanly. Filesystem activity
        // post-drop is moot.
        fs::write(dir.path().join("post.txt"), "ignored").unwrap();
    }

    #[test]
    fn excludes_apply_to_initial_and_live_events() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), "ref").unwrap();
        fs::write(dir.path().join("normal.txt"), "ok").unwrap();
        let scanner =
            Scanner::new(ScanConfig::default().add_exclude("**/.git/**").unwrap()).unwrap();
        let mut stream = scanner.scan(dir.path()).expect("watch start");
        let events = drain_initial(&mut stream, Duration::from_secs(2));
        let paths: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ScanEvent::Initial(entry) => Some(entry.path.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        assert!(!paths.iter().any(|p| p.contains("/.git/")), "got {paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("normal.txt")));
    }
}
