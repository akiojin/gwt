#!/usr/bin/env python3
"""Issue #4346: ten lock checks with three notification-controlled load workers.

Run from any directory. Its four cargo processes intentionally reproduce the
concurrent test load in the acceptance scenario.
"""

import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent
TARGET = "managed_assets::tests::pm_repoint_transaction_holds_materializer_lock_through_callback"
WORKER = "managed_assets::tests::pm_repoint_transaction_lock_stress_worker"
CARGO = ["cargo", "test", "-p", "gwt", "--lib", "--all-features"]
HARNESS = ["--test-threads=1", "--format=pretty", "--color=never"]
RUN_SECONDS = 540


def checked_result(process, log):
    output = log.read_text(encoding="utf-8", errors="replace")
    result = re.search(r"test result: ok\. (\d+) passed; 0 failed; 0 ignored;", output)
    if process.returncode != 0 or result is None:
        raise RuntimeError(f"{log.name}: failed or missing successful test result")
    passed = int(result[1])
    if passed != 1:
        raise RuntimeError(f"{log.name}: expected one exact test, got {passed}")
    return passed


def stop(process):
    if process.poll() is not None:
        return
    if os.name == "nt":
        # Terminating cargo alone leaves its test binary and Git children alive.
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass  # The process may finish between poll() and killpg().
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        if os.name == "nt":
            process.kill()
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.wait()


def run_stress(directory):
    processes = []
    workers = []
    failure = None
    worker_errors = []
    stop_file = directory / "stop"
    deadline = time.monotonic() + RUN_SECONDS

    def spawn(label, command, env=None):
        log = directory / f"{label}.log"
        with log.open("wb") as output:
            process = subprocess.Popen(
                command,
                cwd=ROOT,
                env=env,
                stdout=output,
                stderr=subprocess.STDOUT,
                start_new_session=os.name != "nt",
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
            )
        processes.append((process, log))
        return process, log

    def check_load():
        for process, log in workers:
            if process.poll() is not None:
                checked_result(process, log)
                raise RuntimeError(f"{log.name}: load worker ended before its stop notification")
        if time.monotonic() >= deadline:
            raise RuntimeError(f"stress scenario exceeded {RUN_SECONDS}s")

    try:
        # Warm the shared build before starting the four concurrent processes.
        print("Building the managed asset lock stress tests", flush=True)
        build, _ = spawn("build", CARGO + ["--no-run"])
        if build.wait(timeout=max(0.01, deadline - time.monotonic())) != 0:
            raise RuntimeError("stress test build failed")
        ready_files = [directory / f"{index}.ready" for index in range(1, 4)]
        for index, ready in enumerate(ready_files, 1):
            env = os.environ.copy()
            env["GWT_MANAGED_ASSETS_STRESS_READY"] = str(ready)
            env["GWT_MANAGED_ASSETS_STRESS_STOP"] = str(stop_file)
            workers.append(spawn(
                f"load-{index}", CARGO + [WORKER, "--", "--ignored", "--exact"] + HARNESS, env
            ))
        # Ready is emitted only after a worker has exercised the real lock.
        # Workers keep looping until the finally block sends their stop signal.
        while True:
            check_load()
            if all(ready.is_file() for ready in ready_files):
                break
            time.sleep(0.05)

        for iteration in range(1, 11):
            check_load()
            target, log = spawn(f"target-{iteration}", CARGO + [TARGET, "--", "--exact"] + HARNESS)
            while target.poll() is None:
                check_load()
                time.sleep(0.05)
            check_load()
            checked_result(target, log)
            print(f"PASS {iteration}/10: exact lock test with three active workers", flush=True)
    except BaseException as error:
        failure = error
    finally:
        stop_file.touch()
        # Even when a target fails, let every worker finish and observe its
        # result. Force process-tree cleanup only on an error or timeout.
        for process, log in workers:
            try:
                process.wait(timeout=max(0.01, deadline - time.monotonic()))
                checked_result(process, log)
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                worker_errors.append(f"{log.name}: {error}")
        for process, _ in processes:
            stop(process)
    if failure is not None or worker_errors:
        for _, log in processes:
            print(f"--- {log.name} ---\n{log.read_text(encoding='utf-8', errors='replace')}", flush=True)
        if worker_errors:
            print("\n".join(worker_errors), flush=True)
        if failure is not None:
            raise failure
        raise RuntimeError("load workers did not all pass")


def main():
    with tempfile.TemporaryDirectory(prefix="gwt-managed-assets-stress-") as directory:
        run_stress(Path(directory))
    print("PASS: 10 consecutive exact lock tests; all three load workers passed", flush=True)


if __name__ == "__main__":
    main()
