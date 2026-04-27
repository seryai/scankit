//! Walk a directory tree, applying conventional excludes
//! (`.git`, `node_modules`, `.DS_Store`, build outputs), and print
//! one line per file: size in bytes + relative path.
//!
//! ```bash
//! cargo run --example walk -- /Users/me/Documents
//! ```
//!
//! The `watch` feature isn't required for this example — it uses
//! only the default `walk` feature. To exercise the continuous-
//! watch surface, see the (planned) `examples/watch.rs`.

use std::env;
use std::process::ExitCode;

use scankit::{ScanConfig, Scanner};

fn main() -> ExitCode {
    let Some(root) = env::args().nth(1) else {
        eprintln!("usage: walk <path>");
        return ExitCode::FAILURE;
    };

    // Build a Scanner with a conventional exclude set + a 50 MB
    // size cap. The size cap defends against scanning a 10 GB
    // sqlite dump that someone forgot in their Documents folder
    // — the example would otherwise just enumerate it normally,
    // which is fine, but real consumers usually want the cap.
    let config = ScanConfig::default()
        .max_file_size_bytes(50 * 1024 * 1024)
        .add_exclude("**/.git/**")
        .and_then(|c| c.add_exclude("**/node_modules/**"))
        .and_then(|c| c.add_exclude("**/.DS_Store"))
        .and_then(|c| c.add_exclude("**/target/**"))
        .and_then(|c| c.add_exclude("**/.venv/**"))
        .and_then(|c| c.add_exclude("**/__pycache__/**"));

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

    let mut count: u64 = 0;
    let mut total_bytes: u64 = 0;

    for result in scanner.walk(&root) {
        match result {
            Ok(entry) => {
                println!("{:>12}  {}", entry.size_bytes, entry.path.display());
                count += 1;
                total_bytes += entry.size_bytes;
            }
            Err(e) => {
                // Per-entry errors are mostly permission denials
                // on system folders. Log + continue rather than
                // bail — the walk's already inside the user's
                // chosen tree.
                eprintln!("error: {e}");
            }
        }
    }

    eprintln!();
    eprintln!("Scanned: {count} files, {total_bytes} bytes total");
    ExitCode::SUCCESS
}
