#!/usr/bin/env python3

from pathlib import Path
import runpy


def main() -> None:
    shared_script = (
        Path(__file__).resolve().parents[2]
        / "gwt-issue-ops"
        / "scripts"
        / "inspect_issue.py"
    )
    runpy.run_path(str(shared_script), run_name="__main__")


if __name__ == "__main__":
    main()
