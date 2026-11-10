"""Content-free summaries of exact allocator totals and OS process samples.

Pass one capture directory containing allocator.jsonl, runtime-memory-dark.json,
shutdown.json and (for the older tuple schema) symbol-map.json. Stack attribution
is sampled/top-80 only, never an exact heap census. The process series uses wall
time; allocation uses its monotonic clock.
"""
import json
import gzip
from pathlib import Path
import sys


def records(path):
    if path.exists():
        contents = path.read_text(encoding="utf-8-sig")
    else:
        with gzip.open(str(path) + ".gz", "rt", encoding="utf-8-sig") as stream:
            contents = stream.read()
    return [json.loads(line) for line in contents.splitlines()]


def slope(samples, time_key, value_key):
    times = [row[time_key] for row in samples]
    values = [row[value_key] for row in samples]
    mean_t = sum(times) / len(times)
    mean_v = sum(values) / len(values)
    return (sum((t - mean_t) * (v - mean_v) for t, v in zip(times, values))
            / sum((t - mean_t) ** 2 for t in times) * 3600 / 1e6)


def summarize(directory):
    from datetime import datetime, timezone
    allocator = records(directory / "allocator.jsonl")
    runtime = json.loads((directory / "runtime-memory-dark.json").read_text(encoding="utf-8-sig"))
    if not runtime["passed"] or runtime["browser_errors"]:
        raise ValueError("workload did not pass")
    if any(row.get("pid", runtime["gui_pid"]) != runtime["gui_pid"] for row in allocator):
        raise ValueError("allocator snapshots belong to a different process")
    shutdown = json.loads((directory / "shutdown.json").read_text(encoding="utf-8-sig"))
    if not shutdown["owned_gui_stopped"] or shutdown["test_exit_code"] or shutdown["cleanup_errors"]:
        raise ValueError("workload cleanup did not pass")
    process = runtime["rss_samples"]
    symbol_path = directory / "symbol-map.json"
    symbols = json.loads(symbol_path.read_text(encoding="utf-8")) if symbol_path.exists() else {}
    first = allocator[0]
    start = (datetime.fromisoformat(first["timestamp"].replace("Z", "+00:00"))
             if "timestamp" in first else datetime.fromtimestamp(first["unix_timestamp_seconds"], tz=timezone.utc))
    for row in process:
        row["elapsed_seconds"] = (datetime.fromisoformat(row["timestamp"].replace("Z", "+00:00")) - start).total_seconds()
    points = []
    for minute in (0, 30, 60):
        row = min(allocator, key=lambda row: abs(row["elapsed_seconds"] - minute * 60))
        if abs(row["elapsed_seconds"] - minute * 60) > 61:
            raise ValueError(f"missing {minute}-minute allocator snapshot")
        totals = {}
        unknown = 0
        for stack in row["top_stacks"]:
            if isinstance(stack, dict):
                frames = [frame["symbol"] for frame in stack["frames"]]
                weight = stack["estimated_live_bytes"]
            else:
                frames = [symbols.get(str(address), "") for address in stack[3] if address]
                weight = stack[0]
            owner = next((frame.split(" [")[0].rsplit("+0x", 1)[0] for frame in frames
                          if frame.startswith(("gwt::", "gwt_core::"))
                          and "issue_5561_allocator" not in frame
                          and "__rust_" not in frame), "unattributed")
            totals[owner] = totals.get(owner, 0) + weight
            unknown += sum(not frame or frame.startswith("0x") for frame in frames)
        points.append({
            "requested_minute": minute,
            "elapsed_seconds": row["elapsed_seconds"],
            "exact_requested_live_bytes": row["exact_requested_live_bytes"],
            "exact_live_blocks": row["exact_live_blocks"],
            "overflow_samples": row["overflow_samples"],
            "unresolved_frames": unknown,
            "process_console": row.get("process_console"),
            "top_sampled_owners": sorted(totals.items(), key=lambda item: -item[1])[:10],
        })
    # Match the same monotonic observation window and exclude cold startup.
    allocations = [row for row in allocator if 300 <= row["elapsed_seconds"] <= 3601]
    operating_system = [row for row in process if 300 <= row["elapsed_seconds"] <= 3601]
    if len(allocations) < 50 or len(operating_system) < 90:
        raise ValueError("incomplete 5-60 minute series")
    return {
        "unit": "decimal MB/hour",
        "method": "ordinary least squares over process age 5-60 minutes",
        "limits": "Cold ongoing ingest; four deterministic PTYs; changing real checkout history/host load; instrumented System allocator. Neither matched real-model fleet nor a causal GUI budget proof.",
        "allocator_samples": len(allocator), "process_samples": len(process),
        "allocator_duration_seconds": allocator[-1]["elapsed_seconds"],
        "rates": {
            "requested_live_mb_per_hour": slope(allocations, "elapsed_seconds", "exact_requested_live_bytes"),
            "working_set_mb_per_hour": slope(operating_system, "elapsed_seconds", "rss_bytes"),
            "private_mb_per_hour": slope(operating_system, "elapsed_seconds", "private_bytes"),
        },
        "snapshots": points,
    }


if __name__ == "__main__":
    result = summarize(Path(sys.argv[1]))
    print(json.dumps(result, ensure_ascii=False, indent=2))
