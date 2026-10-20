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
    pending_manifest = global_dir / "pending-update" / "manifest.json"
    before_pending_manifest = fingerprint(pending_manifest)
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
    # Exercise the real HOME fallback, but never let fixtures forward their
    # operations to the live gwt Host that launched this verification.
    environment = os.environ.copy()
    for key in (
        "GWT_BIN_PATH", "GWT_BROWSER_URL_FILE", "GWT_HOOK_BIN",
        "GWT_HOOK_FORWARD_TOKEN", "GWT_HOOK_FORWARD_URL", "GWT_PROJECT_ROOT",
        "GWT_REPO_HASH", "GWT_SESSION_ID", "GWT_SESSION_KIND",
        "GWT_SESSION_RUNTIME_PATH", "GWT_WORKTREE_HASH",
        "GWT_AUTONOMOUS_EXECUTION", "GWT_AUTONOMOUS_ISSUE",
    ):
        environment.pop(key, None)
    print("Running: " + " ".join(command), flush=True)
    try:
        result = subprocess.run(command, check=False, env=environment)
    finally:
        config_unchanged = fingerprint(config) == before_config
        pending_unchanged = fingerprint(pending_manifest) == before_pending_manifest
        sentinel_unchanged = fingerprint(sentinel) == before_sentinel
        unchanged = config_unchanged and pending_unchanged and sentinel_unchanged
        sentinel.unlink(missing_ok=True)
        for name, passed in (
            ("config", config_unchanged),
            ("pending-update manifest", pending_unchanged),
            ("sentinel", sentinel_unchanged),
        ):
            print(
                "Real HOME " + name + " contents/mtime: "
                + ("PASS (unchanged)" if passed else "FAIL (changed)"),
                flush=True,
            )
    return result.returncode if unchanged else 1


if __name__ == "__main__":
    raise SystemExit(main())
