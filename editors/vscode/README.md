# kscr VS Code Extension

Language support for `kscr` (`.ks` files).

## Features

- Syntax highlighting via TextMate grammar
- Basic editor configuration (comments / brackets / quotes)
- LSP client (diagnostics + document symbols) via `kscr-lsp`

## LSP Setup (Recommended)

1) Build the language server:

```bash
cd crates/kscr_lsp
cargo build --release
```

2) Point VS Code to the binary (Settings → search `kscr.lsp.serverPath`):

```json
{
  "kscr.lsp.serverPath": "/absolute/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp"
}
```

If `kscr.lsp.serverPath` is empty, the extension will try to run `kscr-lsp` from `PATH`.

## Development and Packaging (VSIX)

Prerequisites: Node.js 20+

```bash
cd editors/vscode
npm ci
npx --yes @vscode/vsce package --dependencies
```

Install the generated `.vsix` in VS Code:

- Command Palette → `Extensions: Install from VSIX...`

## Reference

- Lexer: https://github.com/mako10k/kscr/blob/main/src/lexer.rs
- BNF: https://github.com/mako10k/kscr/blob/main/docs/LanguageBNF.md
- LSP Quick Start: https://github.com/mako10k/kscr/blob/main/docs/LSP_Quick_Start.md
- LSP Design: https://github.com/mako10k/kscr/blob/main/docs/LSPDesign.md
