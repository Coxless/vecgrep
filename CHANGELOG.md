# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial project scaffolding: Cargo project, mise / rust-toolchain pin to 1.93.0,
  rustfmt / clippy config, GitHub Actions CI (fmt / clippy / test).
- CLI skeleton (Step 2): `clap` v4 derive defines the full surface from
  DESIGN §4 (search flags + `index` / `status` / `clean` / `model {list|use|add}`
  subcommands). Unimplemented paths emit a message to stderr and exit 2.
- File walker (Step 3): `src/walk.rs` enumerates candidate files via the
  `ignore` crate, honoring `.gitignore`, hiding dotfiles, not following
  symlinks, applying `-t/--type` (ripgrep type definitions) and `-g/--glob`
  overrides, and skipping binary files via a NUL-byte probe of the first
  8 KiB. Wired into the search/index pipeline in a later step.

### Changed
- Product renamed from `vecgrep` to `vsgrep` (vector semantic) to avoid a
  name collision. The `vg` alias is still planned.
