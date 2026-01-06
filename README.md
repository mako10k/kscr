# kscr (scaffold)

Rust project scaffolding for the lazy functional scripting language described in `docs/`.

## Prerequisites
Install Rust (includes `cargo`) via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Build / Test

```bash
cargo test
cargo run -- help
cargo run -- parse path/to/file.ks
```
