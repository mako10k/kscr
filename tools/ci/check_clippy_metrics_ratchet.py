#!/usr/bin/env python3
import json
import subprocess
import sys
from pathlib import Path


def load_baseline(path: Path) -> dict[str, int]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise SystemExit(f"baseline must be a JSON object: {path}")
    out: dict[str, int] = {}
    for k, v in data.items():
        if not isinstance(k, str) or not isinstance(v, int):
            raise SystemExit(f"baseline values must be int: {path}")
        out[k] = v
    return out


def count_clippy_warnings() -> dict[str, int]:
    wanted = {"clippy::cognitive_complexity", "clippy::too_many_lines"}
    counts = {k: 0 for k in wanted}

    cmd = [
        "cargo",
        "clippy",
        "--workspace",
        "--all-targets",
        "--message-format=json",
        "--",
        "-W",
        "clippy::cognitive_complexity",
        "-W",
        "clippy::too_many_lines",
    ]

    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert proc.stdout is not None
    assert proc.stderr is not None

    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if obj.get("reason") != "compiler-message":
            continue
        msg = obj.get("message") or {}
        code = (msg.get("code") or {}).get("code")
        if code in counts:
            counts[code] += 1

    stderr = proc.stderr.read()
    rc = proc.wait()

    if rc != 0:
        # If clippy itself failed, show stderr to help debugging.
        sys.stderr.write(stderr)
        raise SystemExit(rc)

    # We intentionally ignore stderr (it may contain normal build output)
    return counts


def main() -> int:
    root = (
        subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True)
        .strip()
    )
    baseline_path = Path(root) / "tools" / "ci" / "clippy_metrics_baseline.json"
    baseline = load_baseline(baseline_path)
    current = count_clippy_warnings()

    failed = False
    for key, base in baseline.items():
        cur = current.get(key, 0)
        if cur > base:
            sys.stderr.write(
                f"ERROR: clippy metric grew: {key}: {base} -> {cur} (must not increase)\n"
            )
            failed = True

    if failed:
        sys.stderr.write("\nTip: reduce complexity/lines gradually; then update baseline intentionally.\n")
        sys.stderr.write("     Update baseline with: python3 tools/ci/update_clippy_metrics_baseline.py\n")
        return 1

    print("clippy metrics ratchet: ok", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
