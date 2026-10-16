"""Measure cold hook senders against a private immediate-ack receiver.

This is a controlled transport workload, not a deployed agent fleet test.
Run the ignored Rust measurement child built from the recorded source SHA.
"""

import argparse
import concurrent.futures
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


@contextmanager
def native_receiver(binary):
    child = subprocess.Popen([str(Path(binary).resolve())], stdout=subprocess.PIPE,
                             text=True, creationflags=(
                                 subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0))
    try:
        url = child.stdout.readline().strip()
        if not url.startswith("http://127.0.0.1:"):
            raise RuntimeError("private receiver failed to publish its URL")
        yield url
    finally:
        child.terminate()
        child.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--receiver", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--workers", type=int, default=12)
    parser.add_argument("--interval", type=float, default=5)
    parser.add_argument("--modes", nargs="+", default=["current", "plain"])
    parser.add_argument("--profile", action="store_true")
    parser.add_argument("--budget-ms", type=int)
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    records = []
    with tempfile.TemporaryDirectory(prefix="gwt-hook-5542-") as private_home:
        with native_receiver(args.receiver) as receiver_url:
            child_env = {k: v for k, v in os.environ.items()
                         if not k.startswith("GWT_")}
            child_env.update(HOME=private_home, USERPROFILE=private_home,
                             GWT_MEASURE_HOOK_URL=receiver_url)

            def sample(mode, worker, round_number):
                env = dict(child_env, GWT_MEASURE_HOOK_MODE=mode)
                if args.budget_ms is not None:
                    env["GWT_TEST_BUDGET_HOOK_LIVE_ATTEMPT_MS"] = str(args.budget_ms)
                profile = Path(private_home) / f"{mode}-{worker}-{round_number}.jsonl"
                if args.profile:
                    env["GWT_HOOK_PROFILE_PATH"] = str(profile)
                started = time.perf_counter()
                result = subprocess.run(
                    [binary, "--exact",
                     "daemon_runtime::tests::issue_5542_cold_hook_measurement_child",
                     "--ignored", "--nocapture"],
                    env=env, capture_output=True, text=True, timeout=20,
                    creationflags=(subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0),
                )
                marker = next((line for line in result.stdout.splitlines()
                               if line.startswith("MEASUREMENT ")), None)
                if result.returncode or marker is None:
                    raise RuntimeError(f"measurement child failed: {result.stderr}")
                row = dict(json.loads(marker[12:]), mode=mode, worker=worker,
                           round=round_number,
                           process_ms=(time.perf_counter() - started) * 1000)
                if profile.exists():
                    row["phases"] = [{k: value[k] for k in ("handler", "duration_ms", "status")}
                                     for value in map(json.loads, profile.read_text().splitlines())]
                return row

            started = time.monotonic()
            round_number = 0
            with output.open("w", encoding="utf-8") as raw:
                with concurrent.futures.ThreadPoolExecutor(args.workers) as pool:
                    while time.monotonic() - started < args.seconds:
                        # Reverse order each round to reduce order bias.
                        modes = args.modes if round_number % 2 == 0 else args.modes[::-1]
                        for mode in modes:
                            futures = [pool.submit(sample, mode, i, round_number)
                                       for i in range(args.workers)]
                            for future in futures:
                                record = future.result()
                                records.append(record)
                                raw.write(json.dumps(record) + "\n")
                            raw.flush()
                        round_number += 1
                        remaining = started + round_number * args.interval - time.monotonic()
                        if remaining > 0:
                            time.sleep(min(remaining, max(0, args.seconds
                                                         - (time.monotonic() - started))))
    summary = dict(workers=args.workers, duration_seconds=time.monotonic() - started,
                   rounds=round_number, binary=binary,
                   test_budget_override_ms=args.budget_ms,
                   binary_sha256=hashlib.file_digest(open(binary, "rb"), "sha256").hexdigest(),
                   source_sha=subprocess.check_output(
                       ["git", "rev-parse", "HEAD"], text=True).strip(),
                   receiver_sha256=hashlib.file_digest(
                       open(args.receiver, "rb"), "sha256").hexdigest(),
                   workload="isolated cold-process transport; not a real agent fleet",
                   modes={})
    for mode in args.modes:
        rows = [row for row in records if row["mode"] == mode]
        times = sorted(row["elapsed_ms"] for row in rows)
        summary["modes"][mode] = dict(
            samples=len(rows), failures=sum(not row["ok"] for row in rows),
            p50_ms=times[len(times) // 2],
            p95_ms=times[min(len(times) - 1, int(len(times) * .95))],
            max_ms=max(times),
        )
    output.with_suffix(".summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == "__main__":
    main()
