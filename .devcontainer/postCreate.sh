#!/usr/bin/env bash
set -euo pipefail

# Ensure core Rust tooling is present (image should already include rustup).
if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup not found; installing via rustup.rs" >&2
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

# Keep stable toolchain available + common components.
rustup toolchain install stable --profile minimal >/dev/null
rustup default stable >/dev/null
rustup component add clippy rustfmt >/dev/null

# Tools used by this repo's quality gates.
# These are installed unconditionally; if installation fails, the devcontainer setup fails.
if ! command -v cargo-geiger >/dev/null 2>&1; then
  cargo install cargo-geiger --locked
fi

if ! command -v cargo-udeps >/dev/null 2>&1; then
  cargo install cargo-udeps --locked
fi

# Nightly is needed to *run* udeps: `cargo +nightly udeps`.
rustup toolchain install nightly --profile minimal >/dev/null

# Speed up incremental builds a bit in Codespaces.
rustup component add rust-src >/dev/null 2>&1 || true

echo "postCreate complete: rustc=$(rustc -V 2>/dev/null || true), cargo=$(cargo -V 2>/dev/null || true)"
