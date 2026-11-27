#!/usr/bin/env python3
"""Issue #6997: verify the real caller, then an isolated checkout in Chromium.

Use Node 22 LTS and the repository-pinned Playwright Chromium.
Build target/debug/gwt and target/debug/gwtd before running. Extra arguments
are forwarded to the shared Playwright runner, including verify.run's reporter.
"""

import argparse
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import tempfile
import time
import urllib.request


ROOT = Path(__file__).resolve().parent.parent
GWTD = ROOT / "target/debug/gwtd"


def envelope(operation, params=None, env=None):
    result = subprocess.run(
        [str(GWTD)], cwd=ROOT, env=env, text=True, capture_output=True,
        input=json.dumps({"schema_version": 1, "operation": operation,
                          "params": params or {}}) + "\n",
        timeout=60, check=True,
    )
    payload = json.loads(result.stdout)
    if payload.get("ok") is not True:
        raise RuntimeError(f"{operation} failed: {payload.get('output')}")
    return payload


def doctor(params, env):
    payload = json.loads(envelope("hook.doctor", params, env)["output"])
    proof = payload["preflight"]
    expected = {"schema_version": 1, "context": "current",
                "normal_completion": "normal", "convergence": "converged",
                "result": "proceed", "reasons": []}
    if any(proof.get(key) != value for key, value in expected.items()):
        raise RuntimeError("Managed-browser preflight failed: " + json.dumps(proof))
    if not isinstance(proof.get("diagnostics"), list) or any(
        not item.get("id") or item.get("classification") not in
        ("recovered", "fail_open", "foreign") for item in proof["diagnostics"]
    ):
        raise RuntimeError("Managed-browser diagnostic is unclassified")
    print("Managed-browser preflight: " + json.dumps(proof), flush=True)
    return payload


def stop(child):
    if child is None:
        return
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
    child.wait(timeout=10)


def start_anchor(base, log):
    # Keep one connection across Playwright's per-theme browser contexts. The
    # last browser disconnect otherwise asks the isolated --no-open GUI to exit.
    program = r"""
const url = new URL('/ws', process.argv[1]);
url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
const socket = new WebSocket(url);
const timer = setTimeout(() => { console.error('anchor connection timed out'); process.exit(1); }, 15000);
socket.addEventListener('open', () => { clearTimeout(timer); console.log('ANCHOR_READY'); });
socket.addEventListener('error', () => { console.error('anchor connection failed'); process.exit(1); });
socket.addEventListener('close', () => process.exit(0));
process.stdin.resume();
process.stdin.on('end', () => socket.close());
"""
    child = subprocess.Popen(["node", "-e", program, base], cwd=ROOT,
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=log, text=True)
    try:
        ready, _, _ = select.select([child.stdout], [], [], 20)
        if not ready or child.stdout.readline().strip() != "ANCHOR_READY":
            raise RuntimeError("Persistent GUI connection failed; inspect anchor.log")
        return child
    except BaseException:
        stop(child)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--expect-daemon", action="store_true")
    mode.add_argument("--expect-inherit", action="store_true")
    args, playwright_args = parser.parse_known_args()
    if playwright_args[:1] == ["--"]:
        playwright_args.pop(0)
    has_source = bool(os.environ.get("GWT_VERIFICATION_DAEMON_SOURCE"))
    if has_source != args.expect_daemon:
        raise RuntimeError("Unexpected daemon source presence for requested spawn host")
    for binary in (ROOT / "target/debug/gwt", GWTD):
        if not os.access(binary, os.X_OK):
            raise RuntimeError("Build checkout binaries first: cargo build -p gwt --bin gwt --bin gwtd")

    source_env = dict(os.environ)
    source_env.pop("GWT_BIN_PATH", None)
    source_env.pop("GWT_HOOK_BIN", None)
    resolver = json.loads(envelope("hook.doctor", env=source_env)["output"])
    hook = resolver["managed_hook_bin"]
    if not hook or re.search(r"(^|[/\\])target[/\\].*[/\\]gwtd(?:\.exe)?$", hook):
        raise RuntimeError("Stable managed-hook fallback is missing")
    source_env["GWT_HOOK_BIN"] = hook
    params = {"purpose": "managed_browser", "expected_hook_bin": hook}
    before = doctor({**params, "repair": True}, source_env)
    repo_hash = envelope("issue.monitor.status")["project_store"]["hash"]
    if not re.fullmatch(r"[0-9a-f]{16}", repo_hash):
        raise RuntimeError("Canonical project_store.hash is missing")

    home = Path(tempfile.mkdtemp(prefix="issue-6997-home-", dir=ROOT / "target"))
    state = home / ".gwt/projects" / repo_hash / "project-state"
    state.mkdir(parents=True)
    (state / "pm.json").write_text('{"settings":{"auto_start":false}}')
    (state / "issue-monitor.json").write_text(
        '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
    (home / ".gwt/session.json").write_text(json.dumps({
        "active_tab_id": "issue-6997",
        "tabs": [{"id": "issue-6997", "title": "Issue 6997 verification",
                  "project_root": str(ROOT), "kind": "git"}],
        "recent_projects": [],
    }))
    runtime = Path.home() / ".gwt/runtime"
    if runtime.is_dir():
        (home / ".gwt/runtime").symlink_to(runtime, target_is_directory=True)
    (home / ".codex").mkdir()
    (home / ".codex/config.toml").write_text("")
    gui_env = {key: value for key, value in os.environ.items()
               if not key.startswith(("GWT_", "CODEX_", "CLAUDE_")) and key not in
               ("GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY")}
    gui_env.update(HOME=str(home), USERPROFILE=str(home), CODEX_HOME=str(home / ".codex"),
                   GWT_HOOK_BIN=hook, GWT_BROWSER_URL_FILE=str(home / "url"),
                   GWT_DISABLE_BACKGROUND_INDEX="1", GIT_TERMINAL_PROMPT="0",
                   GH_PROMPT_DISABLED="1")
    gui = runner = anchor = None
    exit_code = 1
    try:
        with (home / "server.log").open("w") as log, (home / "anchor.log").open("w") as anchor_log:
            gui = subprocess.Popen([str(ROOT / "target/debug/gwt"), "--no-tray", "--no-open"],
                                   cwd=ROOT, env=gui_env, stdout=log, stderr=log)
            print(f"Owned checkout GUI PID: {gui.pid}; isolated HOME: {home}", flush=True)
            deadline = time.monotonic() + 120
            base = ""
            while time.monotonic() < deadline:
                if gui.poll() is not None:
                    raise RuntimeError(f"Checkout GUI exited; inspect {home / 'server.log'}")
                try:
                    base = (home / "url").read_text().strip()
                    with urllib.request.urlopen(urllib.request.Request(base, method="HEAD"), timeout=2):
                        break
                except (OSError, ValueError):
                    base = ""
                    time.sleep(0.2)
            if not base:
                raise RuntimeError("Checkout GUI readiness timed out")
            anchor = start_anchor(base, anchor_log)
            with urllib.request.urlopen(base.rstrip("/") + "/styles/tokens.css", timeout=5) as response:
                if response.read() != (ROOT / "crates/gwt/web/styles/tokens.css").read_bytes():
                    raise RuntimeError("Checkout GUI served stale embedded assets")
            after = doctor(params, source_env)
            (home / "preflight.json").write_text(json.dumps({
                "spawn_host": "daemon" if args.expect_daemon else "inherit",
                "before": before["preflight"], "after": after["preflight"],
            }, indent=2))
            runner_env = dict(os.environ)
            runner_env.update(GWT_PLAYWRIGHT_BASE_URL=base,
                              GWT_PLAYWRIGHT_CHECK_HOME=str(home),
                              GWT_PLAYWRIGHT_PROJECT_ROOT=str(ROOT),
                              GWT_PLAYWRIGHT_PROJECT_KEY=repo_hash,
                              GWT_PLAYWRIGHT_ISSUE_6997="1")
            runner = subprocess.Popen([
                "bash", "scripts/run-visual-tests.sh", "issue-6997-preflight-live.spec.ts",
                "--workers=1", "--retries=0", "--headed", *playwright_args,
            ], cwd=ROOT, env=runner_env)
            exit_code = runner.wait(timeout=300)
            if anchor.poll() is not None:
                raise RuntimeError("Persistent GUI connection exited during verification")
    finally:
        stop(runner)
        stop(anchor)
        stop(gui)
        (home / "shutdown.json").write_text(json.dumps({
            "owned_gui_stopped": gui is None or gui.poll() is not None,
            "owned_anchor_stopped": anchor is None or anchor.poll() is not None,
            "test_exit_code": exit_code,
        }))
        print(f"Evidence: {home}", flush=True)
    return exit_code


if __name__ == "__main__":
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f"signal {signum}")

    for handled in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(handled, interrupted)
    raise SystemExit(main())
