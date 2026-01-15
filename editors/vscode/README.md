# kscr VS Code Extension (MVP)

Minimal `kscr` language support for `.ks` files.

## Features

- Syntax highlighting via TextMate grammar
- Basic editor configuration (comments / brackets / quotes)

## Development and Packaging

Prerequisites: Node.js 20+ (required by `@vscode/vsce`)

```bash
cd editors/vscode
npx --yes @vscode/vsce package
```

Install the generated `.vsix` in VS Code:

- Command Palette → `Extensions: Install from VSIX...`

## Reference Specifications

- Lexer: https://github.com/mako10k/kscr/blob/main/src/lexer.rs
- BNF: https://github.com/mako10k/kscr/blob/main/docs/LanguageBNF.md
- Design Notes: https://github.com/mako10k/kscr/blob/main/docs/VSCodeExtension.md
- LSP Design: https://github.com/mako10k/kscr/blob/main/docs/LSPDesign.md
