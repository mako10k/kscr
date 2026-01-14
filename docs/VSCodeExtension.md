# VS Code Extension (.vsix) Notes / Roadmap

This document describes what we should implement for a VS Code extension for the `kscr` language (file extension: `.ks`), starting from a minimal MVP (syntax highlighting) and growing incrementally.

## Goals

- Make `.ks` pleasant to edit in VS Code.
- Ship **syntax highlighting** as an installable `.vsix` first.
- Keep the foundation (structure / scope names / packaging) friendly for future LSP integration (completion, go-to-definition, diagnostics).

## Non-goals (for MVP)

- Precise parser/AST-driven highlighting.
- Formatting/linting/typechecking integration.
- Semantic tokens that depend on import resolution / module boundaries.

## Implementation approaches (decision: TextMate first)

### A. TextMate Grammar (tmLanguage) + language-configuration (recommended)

- Fastest path to a working MVP.
- Minimal dependencies, easy to package.
- “Good enough” highlighting with reasonable effort.

MVP uses this approach.

### B. Semantic Tokens (requires more infrastructure)

- Enables high-precision highlighting based on binding/type/module resolution.
- Requires calling into `kscr` analysis from VS Code (typically via an LSP server).

Consider in the LSP phase.

### C. Tree-sitter

- High quality, but higher long-term maintenance and more complex build/distribution.

Consider only if/when it becomes necessary.

## What to implement for MVP

### 1) Language ID and file association

- Language id: e.g. `kscr`
- File extension: `.ks`
- File icon (optional)

### 2) TextMate grammar (syntax highlighting)

Keep the grammar aligned with:

- Lexer implementation: [src/lexer.rs](src/lexer.rs)
- Spec (BNF): [docs/LanguageBNF.md](docs/LanguageBNF.md)

#### Minimum token categories

- **Comments**
  - Line comment: `-- ...` (until end of line)
  - Block comment: `{- ... -}` (nestable in the lexer)
  - Shebang: first line `#!...` (treated as a comment)
- **Literals**
  - String: `"..."`
  - Char: `'a'` (and escapes, per the lexer)
  - Numbers: integer / float
  - Booleans: `True` / `False`
- **Keywords** (at least those recognized by the lexer)
  - `module`, `where`, `import`, `export`
  - `let`, `in`, `case`, `of`, `do`, `if`, `then`, `else`
  - `type`, `data`, `class`, `instance`
  - `infix`, `infixl`, `infixr`
  - Highlight-only extras (BNF mentions them; lexer may not yet): `deriving`, `qualified`, `as`
- **Operators / punctuation** (grouped rather than exhaustively enumerated)
  - `->`, `<-`, `=>`, `::`, `==`, `/=`, `<=`, `>=`, `&&`, `||`, `++`, `:`
  - `\` (lambda)
  - `` ` `` (backtick infix call)
- **Identifiers (rough heuristic)**
  - lower-case / `_` start: variable/function
  - Upper-case start: type name / data constructor (Haskell-like)

#### Scope naming (guidelines)

TextMate scopes determine theme compatibility. Prefer common scope names:

- Comments: `comment.line.double-dash.kscr`, `comment.block.kscr`
- String: `string.quoted.double.kscr`
- Char: `constant.character.kscr`
- Numbers: `constant.numeric.kscr`
- Booleans: `constant.language.boolean.kscr`
- Control keywords: `keyword.control.kscr` (let/case/do/if, ...)
- Declaration keywords: `keyword.declaration.kscr` (module/import/data/type/class/instance, ...)
- Types/constructors: `entity.name.type.kscr`
- Operators: `keyword.operator.kscr` / `punctuation.definition.operator.kscr`

### 3) language-configuration.json

Provide basic editor experience:

- Comment toggles
  - lineComment: `--`
  - blockComment: `{-` / `-}`
- Brackets
  - `()`, `[]`, `{}`
- autoClosingPairs / surroundingPairs
  - `""`, `''`, `()`, `[]`, `{}`
- indentationRules (optional)
  - After `where`, `let`, `do`, `of` indentation usually increases; keep the rule lightweight.

### 4) Packaging (.vsix)

- Implement as a standard Node.js-based VS Code extension.
- Package `.vsix` using `vsce`.

Typical commands:

- `npm ci`
- `npm run compile` (only if you add TypeScript)
- `npx --yes @vscode/vsce package`

For MVP, the extension can be grammar-only (no `extension.ts`).

## Node.js installation for packaging (no .devcontainer features)

`@vscode/vsce` requires **Node.js 20+**. If the container image only has Debian’s Node 18 (or no Node at all), install Node 20 from the network.

### Debian/Ubuntu (recommended: NodeSource)

1) Remove existing Debian-provided Node/npm (optional but helps avoid conflicts):

```bash
sudo apt-get remove -y nodejs npm
sudo apt-get autoremove -y
```

2) Add NodeSource repository for Node 20, then install:

```bash
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt-get install -y nodejs
```

3) Verify:

```bash
node -v
npm -v
npx --version
```

### Alternative: nvm

If you prefer not to touch system packages, you can use `nvm` to install Node 20 in your user environment.

## Recommended directory structure

If we keep this in the same repo, separate language implementation from editor tooling:

- `editors/vscode/`
  - `package.json`
  - `language-configuration.json`
  - `syntaxes/kscr.tmLanguage.json`
  - `README.md`

## Notes / gotchas (kscr-specific)

- Fully correct nested block comments (`{- -}`) are hard to express in TextMate; MVP highlighting may approximate.
- `.` is used for module qualification (`A.B`), so tokenization/visual split matters.
- ``a `f` b`` uses backticks for infix calls; highlighting inside backticks improves readability.

## Future work (post-MVP)

Suggested priority order:

1. Snippets
   - `module ... where`
   - `main = do ...`
   - `case ... of`
   - `data ... = ... deriving (...)`
2. Formatter integration (after `kscr fmt` exists)
   - VS Code runs external command on save
3. LSP (primary goal)
   - Implement `textDocument/definition`, `hover`, `diagnostics` via `kscr`
   - Respect kscr module resolution (import base is the importing file’s directory)
4. Semantic highlighting
   - Colorize types/constructors/local bindings/top-level bindings

## Acceptance criteria (Definition of Done for MVP)

- Opening a `.ks` file activates the `kscr` language.
- Comments/strings/numbers/keywords/types are highlighted reasonably.
- `npx --yes @vscode/vsce package` produces a `.vsix` that installs and works in VS Code.

## References

- Keywords (source of truth): [src/lexer.rs](src/lexer.rs)
- Comments / shebang: [docs/LanguageBNF.md](docs/LanguageBNF.md)
- Operators / infix calls: [docs/LanguageBNF.md](docs/LanguageBNF.md)
