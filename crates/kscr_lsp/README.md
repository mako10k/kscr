# kscr-lsp

Language Server Protocol implementation for the kscr language.

## Overview

This crate provides a Language Server Protocol (LSP) server for kscr, enabling IDE features such as:

- **Diagnostics**: Real-time error reporting for parse, import, and type errors
- **Document Symbols**: Outline view of functions, types, and classes
- **Hover** (planned): Show type information on hover
- **Go-to-Definition** (planned): Navigate to symbol definitions
- **Code Completion** (planned): Intelligent code completion
- **Semantic Tokens**: Semantic highlighting tokens (`textDocument/semanticTokens/full`, `.../range`, `.../full/delta`)

## Building

Build the LSP server:

```bash
cd crates/kscr_lsp
cargo build --release
```

The binary will be located at `target/release/kscr-lsp`.

## Usage

The LSP server communicates via stdin/stdout using the JSON-RPC protocol. It is typically started by an editor or IDE extension.

### Manual Testing

You can manually test the server using the LSP protocol. The server expects JSON-RPC messages on stdin:

```bash
./target/release/kscr-lsp
```

## Integration

### VS Code

See the VS Code extension in `editors/vscode` for integration details.

### Other Editors

The LSP server follows the Language Server Protocol specification and should work with any LSP-compatible editor:

- **Vim/Neovim**: Use with `vim-lsp` or `nvim-lspconfig`
- **Emacs**: Use with `lsp-mode` or `eglot`
- **Sublime Text**: Use with `LSP` package

## Architecture

The LSP server consists of three main components:

1. **VFS (Virtual File System)**: Manages document state, including unsaved changes
2. **Backend**: Implements LSP protocol handlers (diagnostics, hover, etc.)
3. **Analysis**: Uses kscr's lexer, parser, and type checker for language analysis

## Development

### Running Tests

```bash
cargo test
```

### Debugging

Enable debug logging by setting the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug ./target/release/kscr-lsp
```

## Current Limitations

- Hover and go-to-definition are not yet implemented
- Diagnostics report errors at line 0 (position extraction from error messages not yet implemented)
- Documents must be saved to disk for type checking (VFS-only typechecking not yet supported)
- No support for workspace-level analysis
- Delta responses currently fall back to full token payloads (no incremental edit list yet)

## Future Enhancements

See `docs/LSPDesign.md` in the main repository for the full roadmap.

Phase 2:
- Hover with type information
- Go-to-definition across modules
- Better error position reporting

Phase 3:
- Code completion
- Find references
- Rename refactoring
- Semantic tokens
