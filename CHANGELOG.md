# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Embedding runtime (Step 5): `src/embed.rs` wraps `ort` (ONNX Runtime, `load-dynamic`)
  and `tokenizers` (HuggingFace) to produce 384-dim f32 vectors (DESIGN §7.1).
  `Embedder::new` loads `model.onnx` + `tokenizer.json` from `VSGREP_MODEL_DIR` and
  asserts dim == 384 via a warmup inference pass.  `embed_query` prefixes `"query: "`;
  `embed_passages` prefixes `"passage: "` and batches at size 64 (configurable).
  Output vectors are L2-normalised so cosine similarity reduces to a dot product
  (Step 6 brute-force will rely on this).  Instrumented with `model.load`,
  `query.tokenize`, `query.embed`, `tokenize.batch`, and `embed.batch` tracing spans
  (DESIGN §11.1); subscriber installed in Step 9.  `#[allow(dead_code)]` — wired into
  the search/index pipeline in Step 8.  Integration tests in `tests/embed_e5.rs` are
  `#[ignore]`-gated; see `tests/fixtures/README.md` for model download instructions.
  fp16 storage deferred to Step 10; model auto-download deferred to Step 16 (M3).
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
- Sliding-window line chunker (Step 4): `src/chunk.rs` produces
  `(path, start_line, end_line, text)` records using a configurable
  line-count window (default 20 lines / 5 overlap, DESIGN §8.1).
  Handles LF + CRLF, rejects non-UTF-8 and files larger than 10 MiB
  (configurable), and drops whitespace-heavy windows (<10% non-WS chars
  or fewer than 2 non-WS lines). Instrumented with a `chunk` tracing
  span so `chunks/sec` is derivable once a subscriber is installed in
  Step 9. Not yet wired into the search/index commands.

### Changed
- Product renamed from `vecgrep` to `vsgrep` (vector semantic) to avoid a
  name collision. The `vg` alias is still planned.
