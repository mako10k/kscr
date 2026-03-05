# Quick Start: Using kscr LSP

This guide gets you started with the kscr Language Server in 5 minutes.

## 1. Build the LSP Server

```bash
cd crates/kscr_lsp
cargo build --release
```

The binary will be at: `target/release/kscr-lsp`

Provisioning note:
- The VS Code extension does not auto-download `kscr-lsp` in the current baseline.
- Provide the server via `kscr.lsp.serverPath` or ensure `kscr-lsp` is available on `PATH`.

## 2. Test It Works

```bash
# Quick smoke test
bash test_lsp.sh
```

You should see:
```
✓ LSP server binary exists and is executable
✓ LSP server can start
LSP server test passed!
```

## 3. Configure Your Editor

### VS Code (Quickest)

1. Install the "kscr" extension from `editors/vscode` (if available)
2. Or manually add to settings.json:

```json
{
  "kscr.lsp.serverPath": "/absolute/path/to/kscr/crates/kscr_lsp/target/release/kscr-lsp"
}
```

### Neovim (With nvim-lspconfig)

Add to your `init.lua`:

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

if not configs.kscr then
  configs.kscr = {
    default_config = {
      cmd = {'/absolute/path/to/kscr-lsp'},
      filetypes = {'kscr'},
      root_dir = lspconfig.util.root_pattern('.git'),
    },
  }
end

lspconfig.kscr.setup{}
```

Add filetype detection to `~/.config/nvim/ftdetect/kscr.vim`:

```vim
au BufRead,BufNewFile *.ks set filetype=kscr
```

## 4. Test IDE Features

Open any `.ks` file and try:

### Test Diagnostics

Create a file with an error:

```haskell
module Test where
  -- Missing equals sign (syntax error)
  x 42
```

You should see a red underline and error message.

### Test Document Symbols

Create a proper file:

```haskell
module Example where
  export add, Person
  
  data Person = Person String Integer
  
  add :: Integer -> Integer -> Integer
  add x y = x + y
```

Check your editor's outline/symbols view - you should see `add` and `Person`.

## 5. Verify Everything Works

✅ Syntax errors appear in real-time  
✅ Type errors are shown (file-backed typecheck)  
✅ Document outline works  
✅ Open-buffer changes are tracked for diagnostics and symbols  

## Troubleshooting

**Problem**: No diagnostics appear

**Solutions**:
1. Check file extension is `.ks`
2. Verify LSP server is running (check editor's LSP status)
3. Look at LSP logs (varies by editor)

**Problem**: LSP server won't start

**Solutions**:
1. Ensure binary is executable: `chmod +x kscr-lsp`
2. Test manually: `echo '{}' | ./kscr-lsp`
3. If VS Code is used, set `kscr.lsp.serverPath` explicitly or ensure `kscr-lsp` is on `PATH`
4. Run `Kscr: Restart LSP` from the command palette after changing server path settings
5. Check for error messages in editor logs

## Next Steps

- See `docs/LSP_Usage.md` for detailed editor configurations
- See `crates/kscr_lsp/README.md` for LSP architecture
- See `docs/LSP_Implementation_Summary.md` for full implementation details

## Example Session

```bash
# Build LSP
cd crates/kscr_lsp
cargo build --release

# Test it
bash test_lsp.sh
# ✓ LSP server test passed!

# Open editor with .ks file
nvim example.ks
# or
code example.ks

# Start coding - diagnostics appear automatically!
```

## What You Get

- **As you type**: Parse errors appear immediately
- **On save**: Type checking runs automatically
- **Outline view**: See all functions and types
- **Also available**: Hover, go-to-definition, completion, references, rename, semantic tokens

That's it! You're ready to use kscr with IDE features.
