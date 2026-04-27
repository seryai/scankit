//! Continuously scan a directory: emit one line per file during
//! the initial walk, then keep printing events as files are
//! created / modified / deleted in the watched tree.
//!
//! Requires the `watch` feature (off by default — pulls in
//! `notify`):
//!
//! ```bash
//! cargo run --example watch --features watch -- /Users/me/Documents
//! ```
//!
//! Stop with Ctrl-C. Drop a new file into the watched folder
//! while the example is running and you'll see the corresponding
//! `Created` event arrive within a few hundred milliseconds.

use std::env;
use std::process::ExitCode;

use scankit::{ScanConfig, ScanEvent, Scanner};

fn main() -> ExitCode {
    let Some(root) = env::args().nth(1) else {
        eprintln!("usage: watch <path>");
        eprintln!();
        eprintln!("Builds with --features watch.");
        return ExitCode::FAILURE;
    };

    // Conventional excludes match the `walk` example so the two
    // are directly comparable. The size cap doesn't apply to
    // notify-driven events for `Deleted` (the file is gone), but
    // does for `Created` / `Modified`.
    let config = ScanConfig::default()
        .max_file_size_bytes(50 * 1024 * 1024)
        .add_exclude("**/.git/**")
        .and_then(|c| c.add_exclude("**/node_modules/**"))
        .and_then(|c| c.add_exclude("**/.DS_Store"))
        .and_then(|c| c.add_exclude("**/target/**"));

    let config = match config {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let scanner = match Scanner::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("scanner error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let stream = match scanner.scan(&root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("watch error: {e}");
            return ExitCode::FAILURE;
        }
    };

    eprintln!("watching {root}; Ctrl-C to stop");
    eprintln!();

    // ScanStream is an Iterator. The blocking next() is the
    // ergonomic shape — the example just sits in this loop
    // forever (until Ctrl-C closes the channel).
    for event in stream {
        match event {
            ScanEvent::Initial(entry) => {
                println!("[init]    {}", entry.path.display());
            }
            ScanEvent::InitialComplete => {
                println!();
                println!("=== initial walk complete; now watching live ===");
                println!();
            }
            ScanEvent::Created(entry) => {
                println!("[created] {}", entry.path.display());
            }
            ScanEvent::Modified(entry) => {
                println!("[mod]     {}", entry.path.display());
            }
            ScanEvent::Deleted(path) => {
                println!("[deleted] {}", path.display());
            }
            // ScanEvent is #[non_exhaustive] — wildcard so future
            // variants (Renamed in v0.3+) don't fail to compile.
            other => {
                println!("[other]   {other:?}");
            }
        }
    }

    ExitCode::SUCCESS
}
