# scankit

**Walk + watch + filter directory trees.** The shared scanner Tauri /
Iced / native desktop apps reach for when they need to enumerate
user files.

> **Status:** v0.2 — one-shot directory walking + continuous
> filesystem-event monitoring. The `walk` feature (default) gives
> you `Scanner::walk(root) -> impl Iterator<Item = ScanEntry>`;
> the `watch` feature (opt-in) adds `Scanner::scan(root) ->
> ScanStream` for an initial walk followed by live Created /
> Modified / Deleted events, with the same exclude + size-cap
> filters applied throughout.

## Why this exists

Every "index files on the user's machine" project — RAG tools,
search apps, backup utilities, file watchers, document assistants —
rebuilds the same five hundred lines of `walkdir`-with-excludes-and-
size-cap-and-symlink-handling glue. Every project gets it slightly
wrong:

- Missed `**/.git/**` in the exclude set, scanned 200K objects in
  a `.pack` file.
- Forgot to cap file sizes, OOM'd on a 50 GB sqlite database the
  user accidentally dropped in their Documents folder.
- Followed a symlink loop and hung the indexer.
- Rebuilt the `GlobSet` per-iteration, ate 30 % of CPU on glob
  compilation alone.

`scankit` ships these bits once, with the edge cases handled
in one place. It's deliberately **lower-level** than a full
indexer — it does not parse files, generate embeddings, or
persist anything. It hands you `ScanEntry`s and gets out of the
way. Pair it with [`mdkit`](https://crates.io/crates/mdkit) for
documents → markdown, with `calamine` / `csv` for tabular files,
with whatever you like for the rest.

## Quick start

```rust
use scankit::{Scanner, ScanConfig};
use std::path::Path;

let scanner = Scanner::new(
    ScanConfig::default()
        .max_file_size_bytes(50 * 1024 * 1024) // 50 MB cap
        .add_exclude("**/.git/**")?
        .add_exclude("**/node_modules/**")?
        .add_exclude("**/.DS_Store")?,
)?;

for result in scanner.walk(Path::new("/Users/me/Documents")) {
    match result {
        Ok(entry) => println!(
            "{}: {} bytes, .{}",
            entry.path.display(),
            entry.size_bytes,
            entry.extension,
        ),
        Err(e) => eprintln!("scan error: {e}"),
    }
}
# Ok::<(), scankit::Error>(())
```

## Design principles

1. **Do one thing well.** Walk + filter + emit `ScanEntry`. Anything
   richer (parse, embed, persist) is the consuming application's
   job.
2. **`Send + Sync` everywhere.** A single `Scanner` shared across
   threads, a single `GlobSet` built once.
3. **No surprises in the iterator.** Filtered-out entries are
   silently dropped. Errors come through as `Err` items in the
   stream — callers can log-and-continue or short-circuit.
4. **Forward-compat defaults.** `ScanConfig` and `ScanEntry` are
   `#[non_exhaustive]` so we can add fields (content hash, inode,
   per-entry metadata) without breaking downstream callers.
5. **Honest dep budget.** `walkdir` + `globset` + `thiserror` are
   the only required deps. `notify` is gated behind the `watch`
   feature.

## Feature flags

| Feature | Adds | Approx. cost |
|---|---|---|
| `walk` (default) | One-shot directory walking | ~250 KB compiled |
| `watch` | Continuous filesystem-event monitoring on top of an initial walk | ~500 KB compiled |
| `default` | `walk` | ~250 KB compiled |

## Examples

Runnable example programs live in [`examples/`](examples/):

- [`walk.rs`](examples/walk.rs) — walk a directory tree with
  conventional excludes (`.git`, `node_modules`, `.DS_Store`,
  build outputs) and a 50 MB size cap. Run with:
  ```bash
  cargo run --example walk -- /Users/me/Documents
  ```

A `watch.rs` example exercising the v0.2 continuous-watch surface
lands in a follow-up release.

## License

Dual-licensed under [MIT](LICENSE-MIT) OR [Apache 2.0](LICENSE-APACHE)
at your option. SPDX: `MIT OR Apache-2.0`.

## Status & roadmap

- [x] **v0.1 — one-shot walking.** `Scanner` + `ScanConfig` +
      `ScanEntry`, exclude-glob and size-cap filters, symlink
      handling, lazy iterator.
- [x] **v0.2 — `watch` feature.** `Scanner::scan` →
      `ScanStream` (an `Iterator<Item = ScanEvent>`). Initial walk
      + continuous filesystem-event monitoring via `notify`, same
      exclude + size-cap filters apply to both. `InitialComplete`
      sentinel marks the boundary between the initial enumeration
      and live events.
- [ ] v0.3 — `Renamed` event variant (consolidate `Deleted` +
      `Created` pairs from notify's platform-specific rename
      shapes); extension-based dispatch helper.
- [ ] v0.4 — audit pass + first stable trait release (1.0
      candidate).

Issues, PRs, and design discussion welcome at
<https://github.com/seryai/scankit/issues>.

## Used by

`scankit` was extracted from the folder-scanner of [Sery
Link][sery], a privacy-respecting data network for the files on
your machines. If you use `scankit` in your project, please open
a PR to add yourself here.

## Acknowledgements

- [`walkdir`](https://crates.io/crates/walkdir) — `BurntSushi`'s
  battle-tested directory walker. Loop detection, permission
  handling, and Send-iterator semantics all come from there.
- [`globset`](https://crates.io/crates/globset) — also `BurntSushi`'s.
  The compiled multi-pattern glob matcher that makes our exclude
  set efficient even with hundreds of patterns.
- [`notify`](https://crates.io/crates/notify) — the cross-platform
  filesystem-event crate that v0.2's watch loop will be built on.
- [`mdkit`](https://crates.io/crates/mdkit) — sibling crate;
  scankit does "files → entries", mdkit does "documents → markdown".

[sery]: https://sery.ai
