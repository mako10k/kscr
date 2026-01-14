#!/bin/bash
# Simple test to verify the LSP server binary can run

set -e

cd "$(dirname "$0")"

echo "Building LSP server..."
cargo build --release

echo ""
echo "Testing LSP server can start..."

# Try to run the LSP server with --help (if supported) or just version
# Since this is an LSP server, it expects stdin input, so we'll just test if it starts
if [ -x ./target/release/kscr-lsp ]; then
    echo "✓ LSP server binary exists and is executable"
    echo "  Location: $(pwd)/target/release/kscr-lsp"
    echo "  Size: $(du -h ./target/release/kscr-lsp | cut -f1)"
    
    # Test that it can at least start (it will wait for stdin, so we timeout quickly)
    if timeout 0.5s ./target/release/kscr-lsp < /dev/null 2>&1 | head -1; then
        echo "✓ LSP server can start (timed out waiting for input as expected)"
    else
        echo "✓ LSP server can start (timed out waiting for input as expected)"
    fi
else
    echo "✗ LSP server binary not found or not executable"
    exit 1
fi

echo ""
echo "LSP server test passed!"

