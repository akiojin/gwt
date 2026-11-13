#!/usr/bin/env python3
"""Build exact consumer artifacts, run the Windows probe, and seal provenance."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile


def run(command, root):
    return subprocess.run(command, cwd=root, text=True, encoding="utf-8", errors="replace", capture_output=True)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expect-red", action="store_true")
    parser.add_argument("--compile-only", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--baseline-ref", help="First measure this archived commit as RED")
    parser.add_argument("--source-head", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if os.name != "nt" and not args.compile_only:
        parser.error("real consumer measurements require Windows; use --compile-only on other hosts")
    if args.baseline_ref and (args.expect_red or args.compile_only):
        parser.error("--baseline-ref requires a full GREEN run")
    root = Path(__file__).resolve().parents[4]
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    if not target.is_absolute():
        target = root / target
    directory = target / "issue6474-probe"
    directory.mkdir(parents=True, exist_ok=True)
    report_path = args.output or directory / ("red.json" if args.expect_red else "green.json")
    report_path = report_path.resolve()
    source = Path(__file__).with_name("windows_cmd_runner_probe.rs")
    executable = directory / ("probe.exe" if os.name == "nt" else "probe")
    baseline_report = None
    if args.baseline_ref:
        baseline_sha = subprocess.check_output(["git", "rev-parse", f"{args.baseline_ref}^{{commit}}"], cwd=root, text=True).strip()
        baseline_report = report_path.with_name(report_path.stem + "-red.json")
        archive = subprocess.check_output(["git", "archive", "--format=zip", baseline_sha], cwd=root)
        with tempfile.TemporaryDirectory(prefix="gwt-6474-red-") as temporary:
            baseline_root = Path(temporary)
            with zipfile.ZipFile(io.BytesIO(archive)) as tree:
                tree.extractall(baseline_root)
            fixture_dir = baseline_root / source.parent.relative_to(root)
            fixture_dir.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, fixture_dir / source.name)
            shutil.copy2(__file__, fixture_dir / Path(__file__).name)
            baseline_env = dict(os.environ, CARGO_TARGET_DIR=str(target))
            subprocess.run([
                sys.executable, str(fixture_dir / Path(__file__).name), "--expect-red",
                "--source-head", baseline_sha, "--output", str(baseline_report),
            ], cwd=baseline_root, env=baseline_env, check=True)
    report = {
        "head": args.source_head or run(["git", "rev-parse", "HEAD"], root).stdout.strip(),
        "source_status": "archived source plus current probe" if args.source_head else run(["git", "status", "--porcelain"], root).stdout,
        "source_diff": "" if args.source_head else run(["git", "diff", "HEAD", "--", "crates"], root).stdout,
        "compiler": run(["rustc", "--version", "--verbose"], root).stdout,
        "compile_only": args.compile_only,
    }
    if baseline_report:
        report["baseline"] = {"ref": args.baseline_ref, "report": str(baseline_report), "sha256": sha256(baseline_report)}
    build_command = ["cargo", "build", "-p", "gwt-agent", "-p", "gwt-terminal", "--lib", "--message-format=json"]
    report["build_command"] = build_command
    build = run(build_command, root)
    report["build_stderr"] = build.stderr
    report["build_exit_code"] = build.returncode
    artifacts = {}
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        name = message["target"]["name"]
        if name not in {"gwt_core", "gwt_agent", "gwt_terminal"}:
            continue
        libraries = [Path(name) for name in message["filenames"] if name.endswith(".rlib")]
        if libraries:
            if name in artifacts and artifacts[name] != libraries[0]:
                raise RuntimeError(f"ambiguous fresh artifact for {name}")
            artifacts[name] = libraries[0]
    code = build.returncode
    if code == 0:
        if set(artifacts) != {"gwt_core", "gwt_agent", "gwt_terminal"}:
            raise RuntimeError(f"missing fresh consumer artifacts: {artifacts}")
        command = ["rustc", "--edition=2021", str(source), "-o", str(executable)]
        for directory_path in sorted({path.parent for path in artifacts.values()}):
            command.extend(["-L", f"dependency={directory_path}"])
        for name, path in sorted(artifacts.items()):
            command.extend(["--extern", f"{name}={path}"])
        report["compile_command"] = command
        compiled = run(command, root)
        report["compile_stderr"] = compiled.stderr
        report["compile_exit_code"] = compiled.returncode
        code = compiled.returncode
    files = [
        source, Path(__file__),
        root / "crates/gwt-core/src/process/windows_resolver.rs",
        root / "Cargo.lock",
        root / "crates/gwt-agent/src/prepare.rs",
        root / "crates/gwt-terminal/src/pty/windows_spawn.rs",
        *artifacts.values(),
    ]
    if code == 0:
        files.append(executable)
        if not args.compile_only:
            command = [str(executable), str(directory / "fixture")]
            if args.expect_red:
                command.append("--expect-red")
            report["run_command"] = command
            measured = run(command, root)
            report.update(exit_code=measured.returncode, stdout=measured.stdout, stderr=measured.stderr)
            code = measured.returncode
            files.extend((directory / "fixture").glob("*.cmd"))
    report["sha256"] = {str(path): sha256(path) for path in files}
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"provenance={report_path} sha256={sha256(report_path)} exit={code}")
    if code:
        print(report.get("compile_stderr", report.get("build_stderr", "")), file=sys.stderr)
        print(report.get("stderr", ""), file=sys.stderr)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
