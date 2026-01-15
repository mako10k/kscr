#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
baseline_file="$root/tools/ci/file_size_baseline.tsv"

warn_threshold_lines=800
hard_threshold_lines=1200

if [[ ! -f "$baseline_file" ]]; then
  echo "ERROR: baseline file not found: $baseline_file" >&2
  exit 2
fi

declare -A baseline
while IFS=$'\t' read -r path lines; do
  [[ "$path" == "path" ]] && continue
  [[ -z "${path:-}" ]] && continue
  baseline["$path"]="$lines"
done < "$baseline_file"

# Current line counts for Rust sources we care about.
mapfile -t current < <(
  find src crates -name '*.rs' -print0 \
    | xargs -0 wc -l \
    | awk 'NF==2 && $2 != "total" && $2 != "合計" { print $2"\t"$1 }' \
    | sort
)

fail=0

# New/changed file policy:
# - New file must stay <= warn_threshold_lines (prevents introducing new mega-files).
# - Existing file:
#   - If it was already > warn_threshold_lines, it must not grow.
#   - If it was <= warn_threshold_lines, it must not cross warn_threshold_lines.
#   - If it was <= hard_threshold_lines, it must not cross hard_threshold_lines.
for entry in "${current[@]}"; do
  path="${entry%%$'\t'*}"
  lines="${entry##*$'\t'}"

  base="${baseline[$path]:-}"

  if [[ -z "${base:-}" ]]; then
    if (( lines > warn_threshold_lines )); then
      echo "ERROR: new Rust file too large: $path is $lines lines (> $warn_threshold_lines)." >&2
      echo "       Please split/refactor before adding large new modules." >&2
      fail=1
    fi
    continue
  fi

  # Ratchet policy (per repo guidance): only enforce the "must not grow" rule
  # once the file is already larger than warn_threshold_lines.
  if (( base > warn_threshold_lines && lines > warn_threshold_lines && lines > base )); then
    echo "ERROR: large file grew: $path ($base -> $lines lines; threshold=$warn_threshold_lines)." >&2
    echo "       Please refactor/split while making changes." >&2
    fail=1
  fi

  if (( base <= warn_threshold_lines && lines > warn_threshold_lines )); then
    echo "ERROR: file crossed size threshold: $path ($base -> $lines; threshold=$warn_threshold_lines)." >&2
    echo "       Please split/refactor before crossing the threshold." >&2
    fail=1
  fi

  if (( base <= hard_threshold_lines && lines > hard_threshold_lines )); then
    echo "ERROR: file crossed hard size limit: $path ($base -> $lines; hard=$hard_threshold_lines)." >&2
    echo "       Please split/refactor." >&2
    fail=1
  fi
done

if (( fail != 0 )); then
  echo "" >&2
  echo "Tip: current largest files (top 10):" >&2
  find src crates -name '*.rs' -print0 \
    | xargs -0 wc -l \
    | sort -nr \
    | head -n 10 >&2
fi

exit "$fail"
