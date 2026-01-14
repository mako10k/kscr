# CI Metrics (Ratchet)

This directory contains small CI checks that help reduce codebase size/complexity gradually.

## File size ratchet

- Script: `tools/ci/check_file_size_ratchet.sh`
- Baseline: `tools/ci/file_size_baseline.tsv`

Policy:

- New Rust files must be **<= 800 lines**.
- Existing files:
  - If the baseline was already **> 800 lines**, the file must **not grow**.
  - If the baseline was **<= 800 lines**, the file must **not cross** 800 lines.
  - If the baseline was **<= 1200 lines**, the file must **not cross** 1200 lines.

This prevents further growth of already-large files and avoids introducing new mega-files, while still allowing gradual refactors to shrink the baseline over time.

Update baseline (only when intentionally resetting after refactors):

```bash
bash tools/ci/update_file_size_baseline.sh
```

## Clippy metrics ratchet

- Script: `tools/ci/check_clippy_metrics_ratchet.py`
- Baseline: `tools/ci/clippy_metrics_baseline.json`

Policy:

- The number of warnings for:
  - `clippy::cognitive_complexity`
  - `clippy::too_many_lines`
  must **not increase** compared to the baseline.

Update baseline (only when intentionally resetting after refactors):

```bash
python3 tools/ci/update_clippy_metrics_baseline.py
```
