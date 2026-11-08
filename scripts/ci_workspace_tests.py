#!/usr/bin/env python3
"""Verify that workspace shards executed the unpartitioned test set exactly once."""
import argparse
import json
from pathlib import Path

from ci_verify_timings import durations, inventory


def verify_shards(root, count):
    expected_paths = {root / f"workspace-tests-{index}" for index in range(1, count + 1)}
    if count < 1 or set(root.glob("workspace-tests-*")) != expected_paths:
        raise ValueError("missing or unexpected workspace shard artifacts")
    baseline, executed, counts = None, set(), []
    for shard in sorted(expected_paths):
        listed = inventory(json.loads(
            (shard / "workspace-test-evidence/inventory.json").read_text(encoding="utf-8")))
        if baseline is None:
            baseline = listed
        elif listed != baseline:
            raise ValueError(f"{shard.name}: unpartitioned inventory or ignored status differs")
        cases = set(durations((shard / "nextest/ci/junit.xml").read_text(encoding="utf-8"), shard.name))
        if executed & cases:
            raise ValueError(f"{shard.name}: tests executed in multiple shards: {sorted(executed & cases)}")
        executed.update(cases)
        counts.append(len(cases))
    expected = {key for key, ignored in baseline.items() if not ignored}
    if executed != expected:
        raise ValueError(f"shard execution differs from unpartitioned inventory: "
                         f"missing={sorted(expected - executed)}, unexpected={sorted(executed - expected)}")
    return {"total": len(baseline), "executed": len(executed),
            "ignored": len(baseline) - len(executed), "per_shard": counts}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--shards", type=int, required=True)
    args = parser.parse_args()
    print(json.dumps(verify_shards(args.root, args.shards), indent=2))
