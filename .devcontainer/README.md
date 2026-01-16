# Dev Container (Codespaces)

This repo supports GitHub Codespaces via `.devcontainer/`.

On creation it:
- Uses the official Rust devcontainer image
- Ensures `clippy` + `rustfmt` are installed
- Tries to install optional quality-gate tools (`cargo-geiger`, `cargo-udeps`)

It also includes:
- GitHub CLI (`gh`) via a devcontainer Feature

Useful commands:

```bash
cargo test
cargo clippy -- -D warnings -D clippy::too_many_lines -D clippy::cognitive_complexity
cargo fmt -- --check
cargo geiger
cargo +nightly udeps
```

Verify that GitHub CLI is available:

```bash
gh --version
```
