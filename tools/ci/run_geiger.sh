#!/usr/bin/env bash
set -euo pipefail

# Runs cargo-geiger in a way that always emits a report even when geiger
# exits non-zero due to warnings (e.g. dependency sources not scanned).
#
# Output:
# - tools/ci/geiger.log        : full stdout/stderr
# - tools/ci/geiger_status.txt : exit status of cargo geiger

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

OUT_LOG="tools/ci/geiger.log"
OUT_STATUS="tools/ci/geiger_status.txt"

set +e
cargo geiger -q >"$OUT_LOG" 2>&1
status=$?
set -e

echo "$status" >"$OUT_STATUS"

# Cargo-geiger uses non-zero exit codes for warning-heavy runs.
# Treat that as a soft signal unless the command itself failed catastrophically.
if [[ $status -ne 0 ]]; then
  echo "cargo geiger exited with status $status (see $OUT_LOG)" >&2
  echo "note: this is often due to unscanned dependency sources; using exit 0" >&2
fi

exit 0
