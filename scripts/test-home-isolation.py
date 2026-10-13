#!/usr/bin/env python3
"""Issue #5339 AC-4: real-home sentinel around the three-crate test matrix.

Only the uniquely named sentinel is created/deleted in the invoking user's
home. Existing settings are fingerprinted, never rewritten or restored.
"""

import hashlib
import os
from pathlib import Path
import subprocess
import tempfile


def fingerprint(path):
    if not path.exists():
        return None
    return hashlib.sha256(path.read_bytes()).hexdigest(), path.stat().st_mtime_ns


def main():
    home = Path(os.environ.get("HOME") or os.environ.get("USERPROFILE") or Path.home())
    global_dir = home / ".gwt"
    global_dir.mkdir(parents=True, exist_ok=True)
    config = global_dir / "config.toml"
    before_config = fingerprint(config)
    with tempfile.NamedTemporaryFile(
        prefix=".test-home-sentinel-5339-", dir=global_dir, delete=False
    ) as fixture:
        fixture.write(b"Issue #5339: tests must leave the real user home unchanged.\n")
        sentinel = Path(fixture.name)
    before_sentinel = fingerprint(sentinel)
    command = [
        "cargo", "test", "-p", "gwt-core", "-p", "gwt", "-p", "gwt-config",
        "--all-features",
    ]
    print("Running: " + " ".join(command), flush=True)
    try:
        result = subprocess.run(command, check=False)
    finally:
        unchanged = (
            fingerprint(config) == before_config
            and fingerprint(sentinel) == before_sentinel
        )
        sentinel.unlink(missing_ok=True)
        print(
            "Real HOME config + sentinel contents/mtime: "
            + ("PASS (unchanged)" if unchanged else "FAIL (changed)"),
            flush=True,
        )
    return result.returncode if unchanged else 1


if __name__ == "__main__":
    raise SystemExit(main())
