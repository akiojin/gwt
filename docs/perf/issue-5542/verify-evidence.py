"""Verify saved transport measurements; this does not certify a deployed fleet."""
import gzip
import json
from pathlib import Path


root = Path(__file__).resolve().parent
for label in ("baseline-paired", "after-paired"):
    summary = json.loads((root / f"{label}.summary.json").read_text())
    assert summary["workers"] == 12, summary
    assert summary["duration_seconds"] >= 3600, summary
    assert summary.get("test_budget_override_ms") is None, "use the binary's actual default"
    assert len(summary["binary_sha256"]) == 64
    with gzip.open(root / f"{label}.jsonl.gz", "rt", encoding="utf-8") as data:
        rows = list(map(json.loads, data))
    for mode, measured in summary["modes"].items():
        samples = [row for row in rows if row["mode"] == mode]
        assert len(samples) == measured["samples"] >= 12 * 600
        failures = sum(not row["ok"] for row in samples)
        assert failures == measured["failures"]
        if label == "after-paired":
            assert failures < 10, (label, mode, failures)
    print(label, summary["modes"])

profile_rows = [json.loads(line) for line in (root / "after-profile.jsonl").read_text().splitlines()]
assert len(profile_rows) >= 12 and all(row["ok"] for row in profile_rows)
phases = {phase["handler"] for row in profile_rows for phase in row["phases"]}
assert {"hook-live/resolve", "hook-live/connect", "hook-live/send", "hook-live/ack"} <= phases
assert "hook-live/client-init" not in phases, "local HTTP must not initialize a cold client"
print("Saved 12-process/60-minute controlled evidence and phase attribution: PASS")
