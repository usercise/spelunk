# Building from Source

Most users should use the precompiled binary — see [Getting Started](getting-started.md).
Build from source if you want to modify spelunk, run the latest unreleased code, or
target a platform without a prebuilt release.

## Prerequisites

### Rust

Install via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Rust 1.80 or later is required (spelunk uses the 2024 edition).

### Inference server

spelunk calls any OpenAI-compatible inference server for embeddings and chat.
[LM Studio](https://lmstudio.ai/) is the recommended option for local use —
download it and load an embedding model before running any commands.
See [Getting Started](getting-started.md#2-set-up-an-inference-server)
for model recommendations and alternative servers (Ollama, vLLM).

## Build

```bash
git clone https://github.com/usercise/spelunk
cd spelunk

# Debug build (faster compile, slower runtime)
cargo build

# Release build (optimised — use this for day-to-day use)
cargo build --release
```

Copy the binary to your `$PATH`:

```bash
cp target/release/spelunk ~/.local/bin/
# or
sudo cp target/release/spelunk /usr/local/bin/
```

Verify:

```bash
spelunk --version
```

## Running tests

```bash
cargo test
```

## Security audit

Requires [cargo-audit](https://crates.io/crates/cargo-audit):

```bash
cargo install cargo-audit
cargo audit
```

## Notes

- The `sqlite-vec` extension is bundled at compile time — no system SQLite extension needed.
- Tree-sitter grammars are compiled as part of the build. If you bump the `tree-sitter` core
  version, check that all `tree-sitter-*` grammar crates are compatible (see `Cargo.toml`).
- Release builds enable LTO and `codegen-units = 1` for a smaller, faster binary.
  Expect a longer compile on first release build.
