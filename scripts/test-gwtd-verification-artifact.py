#!/usr/bin/env python3
"""Issue #4317: pin the state of the operational gwtd build artifact.

`cargo test --all-features` enables the self dev-dependency's `test-gh-guard`
feature, so the `target/debug/gwtd` it leaves behind refuses every real `gh`
spawn. This regression drives the whole sequence against the real artifact:

  1. after a Cargo test run, `target/debug/gwtd` refuses a GitHub read with
     `real_gh_spawn_blocked_in_tests` (the guard is intact -- AC-2), and the
     refusal names the recovery build command (AC-3);
  2. `cargo build -p gwt --bin gwtd` -- the command `verify.run` now appends
     to its own matrix -- restores the operational artifact;
  3. the same path then completes a GitHub read (AC-1).

The GitHub reads use a PATH `gh` fixture under a temporary HOME, and set no
sandbox or live-opt-in marker, so an armed artifact must fail before reaching
the fixture. CI runs this right after `cargo test --workspace --all-features`,
which supplies step 1's armed artifact; run standalone it arms the artifact
itself with one focused integration test.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


CHECKOUT = Path(__file__).resolve().parents[1]
GWTD = CHECKOUT / "target/debug/gwtd"
GUARD_ERROR_CODE = "real_gh_spawn_blocked_in_tests"
RESTORE_COMMAND = ["cargo", "build", "-p", "gwt", "--bin", "gwtd"]
# An integration test needs the gwtd bin built, and --all-features builds it
# with the guard armed. Only used when the artifact is not already armed.
ARM_COMMAND = [
    "cargo", "test", "-p", "gwt", "--all-features",
    "--test", "gwtd_cli_test",
    "gwtd_help_describes_the_headless_cli_surface", "--", "--exact",
]


def run(command):
    print("$ " + " ".join(command), flush=True)
    subprocess.run(command, cwd=CHECKOUT, check=True)


def probe_github_read(root):
    """Invoke the current artifact's pr.checks against a PATH gh fixture.

    Returns (ok, combined output, list of gh argv prefixes actually reached).
    """
    fixture_home, repo, bin_dir = (root / name for name in ("home", "repo", "bin"))
    for directory in (fixture_home, repo, bin_dir):
        directory.mkdir()
    calls = root / "gh-calls.jsonl"
    fake_gh = bin_dir / "gh"
    fake_gh.write_text(
        f"#!{sys.executable}\n"
        "import json, sys\n"
        f"with open({str(calls)!r}, 'a') as log:\n"
        "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
        "if sys.argv[1:3] == ['pr', 'view']:\n"
        "    print(json.dumps({'number': 4317, 'title': 'artifact fixture', "
        "'state': 'OPEN', 'mergeable': 'MERGEABLE', 'mergeStateStatus': 'CLEAN'}))\n"
        "elif sys.argv[1:3] == ['pr', 'checks']:\n"
        "    print(json.dumps([{'name': 'artifact-check', 'state': 'COMPLETED', "
        "'conclusion': 'SUCCESS'}]))\n"
        "else:\n"
        "    raise SystemExit('Unexpected gh call: ' + repr(sys.argv))\n",
        encoding="utf-8",
    )
    fake_gh.chmod(0o755)
    # No sandbox or live-opt-in marker, and no launch or Git context: an armed
    # artifact must refuse before it ever reaches the PATH fixture.
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("GWT_", "GIT_", "GH_", "GITHUB_"))
    }
    env.update(
        HOME=str(fixture_home),
        USERPROFILE=str(fixture_home),
        PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
    )
    subprocess.run(["git", "init", "-q", str(repo)], env=env, check=True)
    subprocess.run(
        ["git", "-C", str(repo), "remote", "add", "origin",
         "https://github.com/fixture/gwt-artifact.git"],
        env=env,
        check=True,
    )
    result = subprocess.run(
        [str(GWTD)],
        input=json.dumps(
            {"schema_version": 1, "operation": "pr.checks", "params": {"number": 4317}}
        ),
        cwd=repo,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    output = result.stdout + result.stderr
    reached = [
        json.loads(line)[:2]
        for line in (calls.read_text().splitlines() if calls.exists() else [])
    ]
    return result.returncode == 0, output, reached


def probe(label):
    with tempfile.TemporaryDirectory(prefix="gwtd-artifact-") as temporary:
        ok, output, reached = probe_github_read(Path(temporary))
    print(f"[{label}] ok={ok} gh calls={reached}", flush=True)
    return ok, output, reached


def main():
    if os.name != "posix":
        raise SystemExit("This artifact regression uses POSIX executable fixtures.")
    assert GWTD.is_file(), "Build first: cargo build -p gwt --bin gwtd"

    ok, output, reached = probe("after cargo test")
    if ok:
        # Standalone run against an already-restored artifact: arm it the way a
        # verification matrix does, then re-probe.
        run(ARM_COMMAND)
        ok, output, reached = probe("after arming cargo test")
    assert not ok, (
        "cargo test --all-features must leave target/debug/gwtd guard-armed, "
        f"but the artifact completed a GitHub read: {output}"
    )
    assert GUARD_ERROR_CODE in output, output
    assert not reached, f"an armed artifact must refuse before spawning gh: {reached}"
    assert " ".join(RESTORE_COMMAND) in output, (
        f"the refusal must name the recovery build command: {output}"
    )

    run(RESTORE_COMMAND)

    ok, output, reached = probe("after restore")
    assert ok, f"the restored artifact must complete a GitHub read: {output}"
    assert "artifact-check" in output, output
    assert reached == [["pr", "view"], ["pr", "checks"]], reached
    print("PASS: cargo test arms target/debug/gwtd; the restore build clears it")


if __name__ == "__main__":
    main()
