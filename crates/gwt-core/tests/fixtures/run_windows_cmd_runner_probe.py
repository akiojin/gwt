"""Run Issue #6474's real health/ConPTY consumers with checkout libraries.

Pass --expect-red before the fix. Cargo checks library freshness and supplies
exact compiler artifacts; the report preserves source/input/binary provenance.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


root = Path(__file__).resolve().parents[4]
source = Path(__file__).with_name("windows_cmd_runner_probe.rs").resolve()
artifacts = root / "target" / "issue6474-probe"
artifacts.mkdir(parents=True, exist_ok=True)
phase = "red" if "--expect-red" in sys.argv else "green"
binary = artifacts / (phase + ".exe")
cargo_command = ["cargo", "build", "-p", "gwt-agent", "-p", "gwt-terminal", "--lib", "--message-format=json"]
build = subprocess.run(cargo_command, capture_output=True, text=True, encoding="utf-8", cwd=root)
sys.stderr.buffer.write(build.stderr.encode("utf-8"))
build.check_returncode()
cargo_artifacts = [
    item for line in build.stdout.splitlines()
    if (item := json.loads(line)).get("reason") == "compiler-artifact"
    and item["target"]["name"] in ["gwt_core", "gwt_agent", "gwt_terminal"]
]
libraries = {
    item["target"]["name"]: Path(next(path for path in item["filenames"] if path.endswith(".rlib")))
    for item in cargo_artifacts
}
assert set(libraries) == {"gwt_core", "gwt_agent", "gwt_terminal"}, cargo_artifacts
# gwt-core is a dependency, so its artifact lives beside the transitive rlibs;
# the explicitly selected gwt-agent/gwt-terminal libraries also have top-level copies.
deps = libraries["gwt_core"].parent
command = ["rustc", "--edition=2021", str(source), "-L", "dependency=" + str(deps), "-o", str(binary)]
for name in ["gwt_agent", "gwt_terminal"]:
    path = libraries[name]
    command.extend(["--extern", name + "=" + str(path)])
subprocess.run(command, check=True, cwd=root)
fixture = artifacts / "fixture"
args = [str(binary), str(fixture)] + (["--expect-red"] if phase == "red" else [])
result = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", cwd=root, timeout=180)
report = {
    "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    "source_status": subprocess.check_output(["git", "status", "--short"], cwd=root, text=True),
    "compiler": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "cargo_command": cargo_command,
    "cargo_artifacts": cargo_artifacts,
    "compile_command": command,
    "run_command": args,
    "sha256": {str(p.relative_to(root) if p.is_relative_to(root) else p): digest(p) for p in [
        source, Path(__file__).resolve(), binary,
        fixture / "fake claude.cmd", *libraries.values(),
        root / "crates/gwt-core/src/process/windows_resolver.rs",
        root / "crates/gwt-agent/src/prepare.rs",
        root / "crates/gwt-terminal/src/pty/windows_spawn.rs",
    ]},
    "exit_code": result.returncode,
    "stdout": result.stdout,
    "stderr": result.stderr,
}
report_path = artifacts / (phase + ".json")
report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
sys.stdout.buffer.write(result.stdout.encode("utf-8"))
sys.stderr.buffer.write(result.stderr.encode("utf-8"))
print("provenance=" + str(report_path.relative_to(root)), "sha256=" + digest(report_path))
sys.exit(result.returncode)
