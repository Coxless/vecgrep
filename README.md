# vsgrep

A ripgrep-like local semantic (vector) search CLI, written in Rust.

> Under construction. See [DESIGN.md](./DESIGN.md) for the target spec and
> [IMPLEMENTATION.md](./IMPLEMENTATION.md) for the step-by-step build plan.

## Status

Phase M1 (Walking Skeleton) — Step 1: project scaffolding.

## Development

Rust toolchain is pinned to **1.93.0** in two places for robustness:

- `.mise.toml` — picked up automatically by [mise](https://mise.jdx.dev/) users.
- `rust-toolchain.toml` — picked up by `rustup` / direct `cargo` invocations, and by CI.

Install the toolchain once, then build:

```bash
mise install            # or: rustup toolchain install 1.93.0
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

## License

MIT. See [LICENSE](./LICENSE).
