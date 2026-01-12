#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Ensure core Rust tooling is present (image should already include rustup).
if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup not found; installing via rustup.rs" >&2
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

# This repo has Git LFS hooks configured in some environments. Install git-lfs
# so commits don't warn/fail due to missing binary.
if ! command -v git-lfs >/dev/null 2>&1; then
  if command -v sudo >/dev/null 2>&1; then
    sudo apt-get update -y >/dev/null
    sudo apt-get install -y git-lfs >/dev/null
  else
    apt-get update -y >/dev/null
    apt-get install -y git-lfs >/dev/null
  fi
fi

# Keep stable toolchain available + common components.
rustup toolchain install stable --profile minimal >/dev/null
rustup default stable >/dev/null
rustup component add clippy rustfmt >/dev/null

# Tools used by this repo's quality gates.
# These are installed unconditionally; if installation fails, the devcontainer setup fails.
required_geiger_version="0.13.0"
if ! command -v cargo-geiger >/dev/null 2>&1; then
  cargo install cargo-geiger --locked --version "$required_geiger_version"
else
  installed_geiger_version="$(cargo-geiger --version | awk '{print $2}')"
  if [[ "$installed_geiger_version" != "$required_geiger_version" ]]; then
    cargo install cargo-geiger --locked --version "$required_geiger_version" --force
  fi
fi

if ! command -v cargo-udeps >/dev/null 2>&1; then
  cargo install cargo-udeps --locked
fi

# Nightly is needed to *run* udeps: `cargo +nightly udeps`.
rustup toolchain install nightly --profile minimal >/dev/null

# Warm the cargo cache so that tools like `cargo geiger` don't need to download
# crates while performing cleanup/scan operations (which can be flaky in some
# container/network setups).
cargo fetch --locked >/dev/null

# Speed up incremental builds a bit in Codespaces.
rustup component add rust-src >/dev/null 2>&1 || true

echo "postCreate complete: rustc=$(rustc -V 2>/dev/null || true), cargo=$(cargo -V 2>/dev/null || true)"
