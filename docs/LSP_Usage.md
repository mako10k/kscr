# Using kscr-lsp with Your Editor

This guide shows how to configure the kscr Language Server with popular editors.

## Building the LSP Server

First, build the LSP server binary:

```bash
cd crates/kscr_lsp
cargo build --release
```

The binary will be at: `crates/kscr_lsp/target/release/kscr-lsp`

## Visual Studio Code

### Option 1: Use the kscr Extension (Recommended)

The VS Code extension in `editors/vscode` includes an LSP client. Build `kscr-lsp` and set `kscr.lsp.serverPath`.

### Option 2: Manual Configuration

Install the `vscode-languageclient` extension and add to your `settings.json`:

```json
{
  "kscr.lsp.enabled": true,
  "kscr.lsp.serverPath": "/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp"
}
```

## Neovim

Using `nvim-lspconfig`:

```lua
-- Add to your init.lua or init.vim
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

-- Define kscr LSP
if not configs.kscr then
  configs.kscr = {
    default_config = {
      cmd = {'/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp'},
      filetypes = {'kscr'},
      root_dir = lspconfig.util.root_pattern('.git', 'Cargo.toml'),
      settings = {},
    },
  }
end

-- Setup kscr LSP
lspconfig.kscr.setup{}
```

Add filetype detection in `ftdetect/kscr.vim`:

```vim
au BufRead,BufNewFile *.ks set filetype=kscr
```

## Vim

Using `vim-lsp`:

```vim
" Add to your .vimrc
if executable('kscr-lsp')
  au User lsp_setup call lsp#register_server({
    \ 'name': 'kscr-lsp',
    \ 'cmd': {server_info->['/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp']},
    \ 'allowlist': ['kscr'],
    \ })
endif

" Filetype detection
au BufRead,BufNewFile *.ks set filetype=kscr
```

## Emacs

Using `lsp-mode`:

```elisp
;; Add to your init.el
(require 'lsp-mode)

(add-to-list 'auto-mode-alist '("\\.ks\\'" . kscr-mode))

(define-derived-mode kscr-mode prog-mode "Kscr"
  "Major mode for kscr files.")

(add-to-list 'lsp-language-id-configuration '(kscr-mode . "kscr"))

(lsp-register-client
 (make-lsp-client :new-connection (lsp-stdio-connection "/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp")
                  :major-modes '(kscr-mode)
                  :server-id 'kscr-lsp))
```

## Sublime Text

Using the `LSP` package:

1. Install the `LSP` package via Package Control
2. Add to `LSP.sublime-settings`:

```json
{
  "clients": {
    "kscr": {
      "enabled": true,
      "command": ["/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp"],
      "selector": "source.kscr"
    }
  }
}
```

3. Add syntax highlighting by creating `Kscr.sublime-syntax` (or use the one from `editors/vscode/syntaxes`)

## Features

Once configured, you'll get:

### ✅ Real-time Diagnostics
- Parse errors
- Type errors
- Import/module resolution errors

### ✅ Document Symbols
- Outline view of functions
- Data type declarations
- Type aliases
- Type classes

### 🔄 Coming Soon
- Hover for type information
- Go-to-definition
- Code completion

## Testing Your Configuration

1. Open a `.ks` file in your editor
2. Introduce a syntax error (e.g., `data Person =`)
3. You should see a diagnostic error appear
4. Check your editor's outline/symbol view to see the document structure

## Troubleshooting

### LSP Server Not Starting

1. Check the binary exists: `ls -l /path/to/kscr-lsp`
2. Make it executable: `chmod +x /path/to/kscr-lsp`
3. Test it manually: `echo '{}' | /path/to/kscr-lsp`

### No Diagnostics Appearing

1. Make sure the file is recognized as a kscr file (`.ks` extension)
2. Check your editor's LSP logs (varies by editor)
3. Ensure the file contains valid module syntax

### Performance Issues

The LSP server currently re-parses and type-checks files on each change. For large files or slow machines, you may notice delays. Future versions will implement incremental parsing and caching.

## Contributing

See the LSP implementation in `crates/kscr_lsp/` and the design document in `docs/LSPDesign.md` for details on contributing to the language server.
