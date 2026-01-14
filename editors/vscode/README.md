# kscr VS Code extension (MVP)

Minimal `kscr` language support for `.ks` files.

## 機能

- Syntax highlighting via TextMate grammar
- Basic editor configuration (comments / brackets / quotes)

## 開発・パッケージング

Prerequisites: Node.js 20+ (required by `@vscode/vsce`)

```bash
cd editors/vscode
npx --yes @vscode/vsce package
```

Install the generated `.vsix` in VS Code:

- Command palette → `Extensions: Install from VSIX...`

## 仕様の参照元

- lexer: https://github.com/mako10k/kscr/blob/main/src/lexer.rs
- BNF: https://github.com/mako10k/kscr/blob/main/docs/LanguageBNF.md
- 設計メモ: https://github.com/mako10k/kscr/blob/main/docs/VSCodeExtension.md
