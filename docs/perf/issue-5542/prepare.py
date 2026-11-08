"""Prepare Windows measurement binaries with the documented Rust 1.99.1 toolchain."""

import json
from pathlib import Path
import shutil
import subprocess


root = Path(__file__).resolve().parents[3]
result = subprocess.run(
    ["cargo", "+1.99.1", "test", "-p", "gwt", "--all-features", "--lib",
     "--no-run", "--message-format=json"],
    cwd=root, text=True, encoding="utf-8", stdout=subprocess.PIPE, check=True,
)
artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
executable = next(
    row["executable"] for row in artifacts
    if row.get("reason") == "compiler-artifact" and row.get("executable")
    and row["target"]["name"] == "gwt" and row["target"]["kind"] == ["lib"]
)
shutil.copy2(executable, root / "target/issue-5542-after.exe")
command = [
    "rustup", "run", "1.99.1", "rustc", "--edition=2021",
    "docs/perf/issue-5542/receiver.rs", "-C", "linker=rust-lld",
    "-L", f"dependency={root / 'target/debug/deps'}",
    "-o", "target/issue-5542-receiver.exe",
]
# Cargo's selected graph identifies compatible crate variants and native paths.
for row in artifacts:
    if row.get("reason") == "build-script-executed":
        for search_path in row.get("linked_paths", []):
            command.extend(["-L", search_path])
for name in ("axum", "tokio"):
    library = next(
        filename for row in reversed(artifacts)
        if row.get("reason") == "compiler-artifact" and row["target"]["name"] == name
        for filename in row["filenames"] if filename.endswith(".rlib")
    )
    command.extend(["--extern", f"{name}={library}"])
subprocess.run(command, cwd=root, check=True)
print("Current checkout measurement sender and private receiver prepared.")
