# Changelog

All notable changes to scankit are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and scankit
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

scankit is pre-1.0 — the public API surface (`Scanner`, `ScanConfig`,
`ScanEntry`, `Error`) is intended to stay stable, but minor versions
may introduce additive changes to feature flags and auxiliary types
until 1.0 lands.

## [Unreleased]

## [0.3.0] — 2026-04-27

### API stability candidate (1.0 prep)

v0.3 is the **API stability candidate** for 1.0. Feature coverage
closed in v0.2 — `walk` for one-shot enumeration, `watch` for
continuous filesystem-event monitoring, both with shared
exclude-glob + size-cap filters. v0.3 freezes the public surface
ahead of 1.0 and locks in SemVer commitments. v0.3.x can iterate
on examples, docs polish, and niche additions without changing
the public API shape.

### Added

- **Stability section in `lib.rs` module docs** explicitly
  enumerates what's covered by the API freeze (Scanner dispatch,
  ScanConfig field set, ScanEntry / ScanEvent / Error
  field+variant sets, the `Iterator<Item = Result<ScanEntry>>`
  shape from `walk`, the `Iterator<Item = ScanEvent>` lifecycle
  from `scan`, feature flag names) and what stays implementation
  detail (private layout of Scanner / ScanWalkIter / ScanStream,
  threading model details, platform-specific event-translation
  rules).

### Changed

- **No API-shape changes.** v0.3.0 is intentionally a
  documentation-only release. `#[non_exhaustive]` was already in
  place on every public struct + enum (added incrementally
  v0.1 → v0.2); `#[must_use]` was already on every constructor +
  builder + accessor. The audit confirmed no gaps.

### Migration

For most callers: bump the dep, rebuild, ship. Zero code changes
required.

### Notes

- v0.3.x will iterate on **more examples** (a recursive-watch
  example with debouncing, a custom-filter example), **cookbook**-
  style docs, and any **niche backend additions** that don't
  change the public surface.
- 1.0 will be cut once the API is exercised by at least one
  downstream production user. Sery Link is the canonical
  integration target.

## [0.2.2] — 2026-04-27

### Added

- **`examples/watch.rs`** — runnable demo of the v0.2 continuous-
  watch surface (`Scanner::scan` → `ScanStream`). Prints one line
  per file during the initial walk, an `=== initial walk complete
  ===` marker when `InitialComplete` arrives, then live
  `[created] / [mod] / [deleted]` events as files change.
  Run with:
  ```bash
  cargo run --example watch --features watch -- /Users/me/Documents
  ```
- `[[example]] required-features = ["watch"]` in `Cargo.toml` so
  `cargo build --all-targets` under the default (walk-only)
  config doesn't try to build the example and fail.

## [0.2.1] — 2026-04-27

### Added

- **`examples/walk.rs`** — runnable CLI that walks a directory
  tree with conventional excludes (`.git`, `node_modules`,
  `.DS_Store`, build outputs) and a 50 MB size cap, printing one
  line per file. Run with:
  ```bash
  cargo run --example walk -- /Users/me/Documents
  ```
- README "Examples" section pointing at the new `examples/`
  directory.

### Notes

- v0.2.1 is the first of the planned "examples + cookbook"
  iteration. A `watch.rs` example exercising the v0.2 continuous-
  watch surface lands in a follow-up release.
- Examples are deliberately dep-light: no `clap` for arg parsing,
  no `serde` for output. Reading the surface should not require
  wading through unrelated crate ceremony.

## [0.2.0] — 2026-04-27

### Added

- **`watch` feature is now real.** v0.1.0 shipped the feature flag
  as a no-op placeholder so consumers could pin against the
  eventual shape; v0.2.0 implements it. Pulls in
  [`notify`](https://crates.io/crates/notify) for the cross-
  platform event source (`FSEvents` on macOS,
  `ReadDirectoryChangesW` on Windows, inotify on Linux).
- **`Scanner::scan(root) -> Result<ScanStream>`** — returns a
  `ScanStream` that does an initial walk + continuous filesystem-
  event monitoring, with the same exclude + size-cap filters as
  `Scanner::walk`. Spawns one short-lived thread for the initial
  walk; the `notify` watcher manages its own threading.
- **`ScanStream`** — `Iterator<Item = ScanEvent>`. Blocking
  `next()` plus a non-blocking `try_next()` for callers that want
  to poll. Dropping the stream stops the underlying watcher and
  releases the kernel-side subscription.
- **`ScanEvent`** enum — `Initial(ScanEntry)`, `InitialComplete`,
  `Created(ScanEntry)`, `Modified(ScanEntry)`, `Deleted(PathBuf)`.
  `#[non_exhaustive]` so `Renamed`, `Closed`, and platform-specific
  kinds can land in v0.3+ without breaking external matches.
- **`InitialComplete` sentinel** — emitted exactly once, after the
  last `Initial` event, before the first live filesystem event.
  Lets UIs flip from "scanning…" to "watching" cleanly.
- **`Error::Watch`** variant — surfaces `notify` initialisation
  failures (root doesn't exist, permission denial, kernel inotify
  budget exhausted on Linux).
- 4 new tests covering: initial walk emits existing files + marker,
  live `Created` events arrive after `InitialComplete`, dropping
  the stream stops the watcher cleanly, excludes apply to both
  initial and live events.

### Notes

- **Renames** are surfaced as a `Deleted(old_path)` followed by a
  `Created(new_entry)` rather than a single `Renamed` variant —
  `notify`'s rename event shape varies by platform and consolidating
  is left for v0.3.
- **Filtering for `Deleted` events.** Excludes are checked, but the
  size cap can't be (the file is gone). If a caller cared about
  the path's excluded-ness they would have skipped it on the
  corresponding `Initial` / `Created` event anyway.
- **Threading model.** The initial-walk thread and `notify`'s
  internal threads both push into one `mpsc::channel`, so events
  for files modified during the initial walk can interleave with
  `Initial` events. Callers that need strict ordering should buffer
  until `InitialComplete` before treating subsequent events as live.

## [0.1.0] — 2026-04-27

### Added

- Initial release. Establishes the crate name on crates.io and the
  public API surface that future watch / dispatch features will
  target.
- **`Scanner`** — the configured walker. Cheap to construct, `Send +
  Sync`, share a single instance across threads.
- **`ScanConfig`** — the policy. Builder-style with
  `max_file_size_bytes(bytes)`, `add_exclude(glob)`,
  `follow_symlinks(bool)`. `#[non_exhaustive]` for forward-compat.
- **`ScanEntry`** — the unit of output. Carries `path`,
  `size_bytes`, `modified`, and a pre-lowercased dot-less
  `extension`. `#[non_exhaustive]` for forward-compat.
- **`Scanner::walk(root)`** — lazy iterator yielding
  `Result<ScanEntry>`. Filtered-out entries (excludes, oversized
  files, directories, symlinks when not following) are silently
  dropped from the iterator. `walkdir` errors surface as `Err`
  items; callers choose log-and-continue vs. short-circuit.
- **Typed `Error` enum** — `InvalidExclude`, `Io`, `Walk`. Coarse-
  grained on purpose; `#[non_exhaustive]` so we can add variants
  in minor versions.
- **Feature flags pre-declared**:
  - `walk` (default) — `walkdir` + `globset` for the one-shot
    walking surface that v0.1.0 ships.
  - `watch` — placeholder for v0.2's continuous-watch feature
    (pulls in `notify`). Existing in v0.1.0 so consumers can pin
    against the eventual shape.
- 14 unit tests covering: default walking, exclude-on-files,
  exclude-on-directories-skipping-subtree, size-cap filtering,
  extension lowercasing, extensionless files, missing root.
- Dual-licensed under MIT OR Apache-2.0 (Rust ecosystem
  convention).
- CI workflow on Ubuntu + macOS + Windows (stable Rust + MSRV
  1.85 + clippy + rustfmt + cargo-audit gates) — same template
  as `mdkit`.
- `CONTRIBUTING.md`, `SECURITY.md` for repo hygiene.

### Notes

- **Why MSRV 1.85.** Matches `mdkit`'s floor — a single MSRV
  across the Sery kit-family means downstream Tauri apps don't
  have to manage divergent Rust toolchains.
- **Why `unsafe_code = forbid` (not `deny`).** Unlike `mdkit`,
  `scankit` has no FFI surface — every backend is pure Rust. Any
  `unsafe` block here is a bug, not a justified opt-in.

[Unreleased]: https://github.com/seryai/scankit/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/seryai/scankit/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/seryai/scankit/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/seryai/scankit/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/seryai/scankit/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/seryai/scankit/releases/tag/v0.1.0
