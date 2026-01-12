# Dev Container (Codespaces)

This repo supports GitHub Codespaces via `.devcontainer/`.

On creation it:
- Uses the official Rust devcontainer image
- Ensures `clippy` + `rustfmt` are installed
- Tries to install optional quality-gate tools (`cargo-geiger`, `cargo-udeps`)

Useful commands:

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt -- --check
cargo geiger
cargo +nightly udeps
```
