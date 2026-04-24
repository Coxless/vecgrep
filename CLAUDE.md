# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**vsgrep** — a semantic (vector) search CLI in Rust, complementing ripgrep for meaning-based queries. All processing is local (no network after initial model download). Current phase: M1 Walking Skeleton (Steps 1–3 of 9 complete).

The design doc is in `DESIGN.md` and the step-by-step roadmap is in `IMPLEMENTATION.md`. Read both before implementing new features.

## Commands

```bash
mise install                                                  # pin Rust 1.93.0 toolchain
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

To run a single test:
```bash
cargo test <test_name>
cargo test walk::tests::binary_file_is_excluded
```

## Architecture

Single binary crate (`src/main.rs` as entry point). Currently three modules:

- **`src/cli.rs`** — Full clap v4 derive-API CLI. Default mode is search (`vsgrep "query" [paths...]`); subcommands: `index`, `status`, `clean`, `model {list|use|add}`. All flags defined here even if unimplemented.
- **`src/walk.rs`** — File candidate enumeration via the `ignore` crate. Honors `.gitignore`, detects binary files (NUL-byte probe in first 8 KiB), supports type filtering and glob overrides. Currently annotated `#[allow(dead_code)]` — not yet wired into `main.rs`.
- **`src/main.rs`** — Routes CLI args; stubs unimplemented subcommands with `eprintln!` + `ExitCode::FAILURE`.

Planned components (not yet implemented, per DESIGN.md):
- **Indexer**: Chunking → embedding → write to `.vsgrep/` store
- **Query engine**: Embed query → HNSW search → optional reranking → format output
- **Model manager**: Default `intfloat/multilingual-e5-small` (384-dim), swappable via `--model`
- **Index store** (`.vsgrep/`): `meta.bin`, `chunks.bin`, `vectors.bin`, `hnsw.bin`, `file_index.bin`

Future crate split: `vsgrep-core` (library) + `vsgrep-cli` (binary wrapper).

## Conventions

- **Toolchain**: Rust edition 2024, MSRV 1.93.0 (pinned in `rust-toolchain.toml` and `.mise.toml`)
- **Line width**: 100 (`rustfmt.toml`)
- **Error types**: `thiserror::Error` with `#[error(...)]` and `#[source]` for chaining; public error enums per module
- **Exit codes**: `ExitCode::FAILURE` (1) for runtime errors, code 2 pattern for CLI errors
- **Tests**: `#[cfg(test)] mod tests` blocks per module; use `tempfile::tempdir()` for filesystem fixtures; CLI tests use a local `parse()` helper
- **Workflow**: one step = one PR; feature branches off `develop`; merge back to `develop`
- **CHANGELOG.md** must be updated in every PR (Keep a Changelog format)
