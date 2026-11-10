#!/usr/bin/env python3
"""Reject project stores leaked into the runner's real HOME by a test job.

Take a snapshot before the suite and check it even when a test step fails.
The guard never creates, deletes, or restores anything in the observed HOME.
"""
import argparse
import json
import os
from pathlib import Path


def project_entries(projects):
    try:
        return sorted(entry.name for entry in projects.iterdir() if entry.is_dir())
    except FileNotFoundError:
        return []


def write_snapshot(state, projects=None):
    if projects is None:
        home = Path(os.environ.get("HOME") or os.environ.get("USERPROFILE") or Path.home())
        projects = home / ".gwt/projects"
    snapshot = {"projects": str(projects.absolute()), "entries": project_entries(projects)}
    state.parent.mkdir(parents=True, exist_ok=True)
    state.write_text(json.dumps(snapshot), encoding="utf-8")
    print(f"Real HOME project stores before tests: {len(snapshot['entries'])}", flush=True)


def check_snapshot(state):
    snapshot = json.loads(state.read_text(encoding="utf-8"))
    before = set(snapshot["entries"])
    after = set(project_entries(Path(snapshot["projects"])))
    added = sorted(after - before)
    print(json.dumps({"real_home_projects": "FAIL" if added else "PASS",
                      "before": len(before), "after": len(after), "added": added}), flush=True)
    if added:
        print("Isolate the leaking fixture with ScopedGwtHome or a temporary HOME/USERPROFILE, "
              "then rerun the job. The guard leaves existing stores untouched.", flush=True)
    return 1 if added else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["snapshot", "check"])
    parser.add_argument("state", type=Path)
    args = parser.parse_args()
    if args.operation == "snapshot":
        write_snapshot(args.state)
        return 0
    return check_snapshot(args.state)


if __name__ == "__main__":
    raise SystemExit(main())
