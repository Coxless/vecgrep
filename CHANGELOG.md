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

### Changed
- Product renamed from `vecgrep` to `vsgrep` (vector semantic) to avoid a
  name collision. The `vg` alias is still planned.
