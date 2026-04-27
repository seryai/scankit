# scankit

**Walk + watch + filter directory trees.** The shared scanner Tauri /
Iced / native desktop apps reach for when they need to enumerate
user files.

> **Status:** v0.3 — **API stability candidate for 1.0**. Feature
> coverage closed in v0.2 (one-shot walking via the `walk` feature,
> continuous filesystem-event monitoring via the `watch` feature,
> both with shared exclude-glob + size-cap filters). v0.3 freezes
> the public surface — see the [stability section](#stability-v03)
> below for what's locked in. v0.3.x will iterate on examples +
> cookbook docs. 1.0 ships once the API is exercised by at least
> one downstream production user.

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
- [`watch.rs`](examples/watch.rs) — continuous scan: initial
  walk + live filesystem events. Requires the `watch` feature.
  Run with:
  ```bash
  cargo run --example watch --features watch -- /Users/me/Documents
  ```

## Stability (v0.3+) {#stability-v03}

v0.3 is the **API stability candidate** for 1.0. The following
surface is committed to and will only change with a major version
bump:

- `Scanner` construction + dispatch — `new`, `walk`, `scan` (under
  the `watch` feature), `config`. Future trait methods land with
  default impls so existing callers don't break.
- `ScanConfig` field set + the builder methods
  (`max_file_size_bytes`, `follow_symlinks`, `add_exclude`).
- `ScanEntry`, `ScanEvent`, `Error` field/variant sets.
  All `#[non_exhaustive]` so we can grow them without major
  bumps. **Pattern-matchers must include a wildcard arm.**
- The lazy `Iterator<Item = Result<ScanEntry>>` shape from
  `Scanner::walk`.
- The `Iterator<Item = ScanEvent>` lifecycle from `Scanner::scan`
  (`Initial` → `InitialComplete` → live `Created` / `Modified` /
  `Deleted`).
- Feature flag names: `walk`, `watch`.

The following are **implementation details** and may change in
minor versions:

- Internal layout of `Scanner` / `ScanWalkIter` / `ScanStream`
  (private fields, helper methods).
- Threading model of `Scanner::scan` (currently one short-lived
  initial-walk thread + the `notify` watcher's own threads).
- Platform-specific event-translation rules (notify itself is
  platform-specific; we follow upstream).

1.0 will be cut once the API is exercised by at least one
downstream production user.

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
- [x] **v0.3 — API stability candidate.** Stability commitments
      doc in `lib.rs` + README. `#[non_exhaustive]` already on
      every public struct + enum (added incrementally v0.1 →
      v0.2); `#[must_use]` already on every constructor +
      builder + accessor. Documentation-only release — no
      API-shape changes.
- [ ] v0.4 — `Renamed` event variant (consolidate `Deleted` +
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
