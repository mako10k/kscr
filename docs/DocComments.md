# Doc Comments (Haskell-style)

This document describes the `kscr` doc comment format used by the compiler and LSP.

## Syntax

- Line doc comment: `-- | ...`
- Block doc comment: `{-| ... -}`

Notes:

- `--` (without `|`) and `{- ... -}` (without `|`) are regular comments and are ignored by the doc system.

## Attachment rule (MVP)

Doc comments attach to the *next* top-level declaration.

Supported targets in MVP:

- Top-level value bindings
- `type` aliases
- `data` declarations
- `class` declarations

Doc comments do **not** attach across an empty line.

If multiple doc comments appear consecutively, their contents are concatenated with `\n`.

## LSP usage

- Hover shows:
  - the inferred type (if available)
  - then the doc comment body as Markdown
- Completion items include doc comment body as Markdown when available.
