# Doc Comments (Haskell-style)

This document describes the `kscr` doc comment format used by the compiler and LSP.

## Syntax

- Line doc comment: `-- | ...`
- Block doc comment: `{-| ... -}`

Notes:

- `--` (without `|`) and `{- ... -}` (without `|`) are regular comments and are ignored by the doc system.

## Attachment rule

Doc comments attach to the next declaration at parser collection points.

Supported targets:

- Top-level value bindings
- `type` aliases
- `data` declarations
- `class` declarations
- `data` constructors (inside `data ... = ...`)

Blank-line behavior follows parser token handling.

- Pending doc text is cleared after two consecutive `Newline` tokens.
- In practice, `{-| ... -}` before a declaration is cleared by one empty line, while `-- | ...` may survive one empty line.

If multiple doc comments appear consecutively, their contents are concatenated with `\n`.

## LSP usage

- Hover shows:
  - the inferred type (if available)
  - then the doc comment body as Markdown
- Completion items include doc comment body as Markdown when available.
