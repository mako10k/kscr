#!/usr/bin/env python3
import json
import subprocess
import sys
from pathlib import Path


def count() -> dict[str, int]:
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

    rc = proc.wait()
    if rc != 0:
        stderr = proc.stderr.read() if proc.stderr is not None else ""
        sys.stderr.write(stderr)
        raise SystemExit(rc)

    return counts


def main() -> int:
    root = (
        subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True)
        .strip()
    )
    out = Path(root) / "tools" / "ci" / "clippy_metrics_baseline.json"
    data = count()
    out.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Updated: {out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
