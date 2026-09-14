#!/usr/bin/env python3
"""Issue #4317: test the actual post-verification gwtd, without GitHub traffic.

Run from outside Cargo; verify.run handles host verification admission. This
registers and runs a focused plan in the current checkout/session; do not
nest this script inside verify.run. Only the subsequent GitHub probe uses a
temporary HOME and repository. Requires a built checkout target/debug/gwtd.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


CHECKOUT = Path(__file__).resolve().parents[1]
GWTD = CHECKOUT / "target/debug/gwtd"
COMMAND = (
    "cargo test -p gwt --all-features --test gwtd_cli_test "
    "gwtd_help_describes_the_headless_cli_surface -- --exact"
)


def invoke(operation, params, cwd, env):
    result = subprocess.run(
        [str(GWTD)],
        input=json.dumps({"schema_version": 1, "operation": operation, "params": params}),
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    response = json.loads(result.stdout)
    assert response["ok"], response
    return response["output"]


def main():
    if os.name != "posix":
        raise SystemExit("This artifact regression uses POSIX executable fixtures.")
    assert GWTD.is_file(), "Build first: cargo build -p gwt --bin gwtd"
    run_env = os.environ.copy()
    run_env.setdefault("GWT_SESSION_ID", "gwtd-artifact-regression")
    run_env.pop("GWT_ALLOW_REAL_GH", None)
    invoke("verify.plan", {"commands": [COMMAND]}, CHECKOUT, run_env)
    transcript = invoke(
        "verify.run",
        {"commands": [COMMAND], "user_verification_result": "n/a"},
        CHECKOUT,
        run_env,
    )
    print(transcript, flush=True)

    with tempfile.TemporaryDirectory(prefix="gwtd-artifact-") as temporary:
        root = Path(temporary)
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
        # No sandbox or live-opt-in marker: an armed artifact must fail before
        # reaching the PATH fixture. Strip launch and Git context as well.
        probe_env = {
            key: value
            for key, value in run_env.items()
            if not key.startswith(("GWT_", "GIT_", "GH_", "GITHUB_"))
        }
        probe_env.update(
            HOME=str(fixture_home),
            USERPROFILE=str(fixture_home),
            PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
        )
        subprocess.run(["git", "init", "-q", str(repo)], env=probe_env, check=True)
        subprocess.run(
            ["git", "-C", str(repo), "remote", "add", "origin",
             "https://github.com/fixture/gwt-artifact.git"],
            env=probe_env,
            check=True,
        )
        output = invoke("pr.checks", {"number": 4317}, repo, probe_env)
        assert "artifact-check" in output, output
        observed = [json.loads(line)[:2] for line in calls.read_text().splitlines()]
        assert observed == [["pr", "view"], ["pr", "checks"]], observed
    assert "$ cargo build -p gwt --bin gwtd\n" in transcript, transcript
    print("PASS: all-features test -> same-path gwtd GitHub read; no guard bypass")


if __name__ == "__main__":
    main()
