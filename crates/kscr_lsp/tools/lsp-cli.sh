#!/usr/bin/env bash
set -euo pipefail

# Minimal LSP CLI for kscr-lsp.
# Requires: jq

server="${1:-}"
file="${2:-}"

if [[ -z "$server" || -z "$file" ]]; then
  echo "Usage: $0 /abs/path/to/kscr-lsp /abs/path/to/File.ks" >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq is required." >&2
  exit 2
fi

uri="file://$file"
text=$(cat "$file")

rpc() {
  local payload="$1"
  local len
  len=$(printf '%s' "$payload" | wc -c)
  printf 'Content-Length: %s\r\n\r\n%s' "$len" "$payload"
}

init=$(jq -nc --arg rootUri "file:///" '  {
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      processId: null,
      rootUri: $rootUri,
      capabilities: {
        textDocument: { publishDiagnostics: {} }
      }
    }
  }')

opened=$(jq -nc --arg uri "$uri" --arg text "$text" '  {
    jsonrpc: "2.0",
    method: "textDocument/didOpen",
    params: {
      textDocument: {
        uri: $uri,
        languageId: "kscr",
        version: 1,
        text: $text
      }
    }
  }')

shutdown=$(jq -nc '{jsonrpc:"2.0", id: 2, method:"shutdown", params:null}')
exitn=$(jq -nc '{jsonrpc:"2.0", method:"exit", params:null}')

(
  rpc "$init"
  rpc "$opened"
  # give the server time to typecheck and publish diagnostics
  sleep 0.2
  rpc "$shutdown"
  rpc "$exitn"
) | "$server" | cat
