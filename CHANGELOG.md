# Changelog

All notable changes to scankit are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and scankit
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

scankit is pre-1.0 — the public API surface (`Scanner`, `ScanConfig`,
`ScanEntry`, `Error`) is intended to stay stable, but minor versions
may introduce additive changes to feature flags and auxiliary types
until 1.0 lands.

## [Unreleased]

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

[Unreleased]: https://github.com/seryai/scankit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/seryai/scankit/releases/tag/v0.1.0
