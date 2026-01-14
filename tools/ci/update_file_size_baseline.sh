#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
out="$root/tools/ci/file_size_baseline.tsv"

echo -e "path\tlines" > "$out"
find src crates -name '*.rs' -print0 \
  | xargs -0 wc -l \
  | awk 'NF==2 && $2 != "total" { print $2"\t"$1 }' \
  | sort >> "$out"

echo "Updated: $out" >&2
