#!/usr/bin/env python3
"""Compare a PR's build profiles on identical sources and dependency versions."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import tomllib


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root)


def base_file(root, base, name):
    exists = subprocess.run(["git", "cat-file", "-e", f"{base}:{name}"], cwd=root,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return git(root, "show", f"{base}:{name}") if exists.returncode == 0 else None


def read_optional(path):
    return path.read_bytes() if path.exists() else None


def parsed(data):
    return tomllib.loads(data.decode("utf-8")) if data is not None else {}


def changes(root, base):
    base = git(root, "rev-parse", "--verify", base).decode().strip()
    manifest = (root / "Cargo.toml").read_bytes()
    previous = base_file(root, base, "Cargo.toml")
    if previous is None:
        raise ValueError("baseline Cargo.toml is missing")
    reasons = []
    if parsed(manifest).get("profile", {}) != parsed(previous).get("profile", {}):
        reasons.append("Cargo profiles changed")
    if parsed(read_optional(root / ".cargo/config.toml")) != parsed(base_file(root, base, ".cargo/config.toml")):
        reasons.append("Cargo configuration changed")
    return base, reasons


def profile_parts(data):
    """Keep TOML table bytes intact; validate the combined document before use."""
    text = data.decode("utf-8")
    headers = list(re.finditer(r"(?m)^\[.+\][ \t]*(?:#.*)?\r?$", text))
    other = [text[:headers[0].start()]] if headers else [text]
    profiles = []
    for index, header in enumerate(headers):
        end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
        block = text[header.start():end]
        (profiles if re.match(r"\[profile(?:\.|\])", header.group()) else other).append(block)
    return "".join(other).encode(), "".join(profiles).encode()


def baseline_manifest(current, previous):
    other, _ = profile_parts(current)
    _, profiles = profile_parts(previous)
    combined = other.rstrip() + b"\n\n" + profiles
    actual = parsed(combined)
    expected = parsed(current)
    expected.pop("profile", None)
    if parsed(previous).get("profile") is not None:
        expected["profile"] = parsed(previous)["profile"]
    if actual != expected:
        raise ValueError("profile replacement would change non-profile settings")
    return combined


def save(output, result):
    (output / "measurement.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


def measure(root, base, output):
    manifest = root / "Cargo.toml"
    config = root / ".cargo/config.toml"
    original_manifest, original_config = manifest.read_bytes(), read_optional(config)
    previous = base_file(root, base, "Cargo.toml")
    restored_manifest = baseline_manifest(original_manifest, previous)
    previous_config = base_file(root, base, ".cargo/config.toml")
    output.mkdir(parents=True, exist_ok=False)
    temp_parent = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())).resolve()
    work = Path(tempfile.mkdtemp(prefix="gwt-build-profile-", dir=temp_parent)).resolve()
    before, after = work / "before", work / "after"
    result = {
        "head": git(root, "rev-parse", "HEAD").decode().strip(), "base": base,
        "rustc": subprocess.check_output(["rustc", "-vV"], cwd=root, text=True).strip(),
        "before_profiles": parsed(previous).get("profile", {}),
        "after_profiles": parsed(original_manifest).get("profile", {}),
        "before_config": parsed(previous_config),
        "after_config": parsed(original_config),
        "size_measurement": "sum of file lengths (hard-linked paths counted separately)",
        "initial_free_bytes": shutil.disk_usage(work).free,
        "source_restored": False, "valid_for_comparison": False, "runs": [],
    }
    save(output, result)
    try:
        subprocess.run(["cargo", "fetch", "--locked"], cwd=root, check=True)
        for variant, target in (("before", before), ("after", after)):
            target.mkdir()
            try:
                if variant == "before":
                    manifest.write_bytes(restored_manifest)
                    if previous_config is None:
                        config.unlink(missing_ok=True)
                    else:
                        config.parent.mkdir(parents=True, exist_ok=True)
                        config.write_bytes(previous_config)
                for temperature in ("cold", "warm"):
                    command = ["cargo", "test", "-p", "gwt", "--no-run", "--locked", "--target-dir", str(target)]
                    with (output / f"{variant}-{temperature}.log").open("w", encoding="utf-8") as log:
                        started = time.perf_counter()
                        completed = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
                        elapsed = time.perf_counter() - started
                    result["runs"].append({
                        "variant": variant, "temperature": temperature, "command": command,
                        "wall_seconds": elapsed, "target_bytes": sum(p.stat().st_size for p in target.rglob("*") if p.is_file()),
                        "exit_code": completed.returncode,
                    })
                    save(output, result)
                    if completed.returncode:
                        raise subprocess.CalledProcessError(completed.returncode, command)
            finally:
                manifest.write_bytes(original_manifest)
                if original_config is None:
                    config.unlink(missing_ok=True)
                else:
                    config.write_bytes(original_config)
            if variant == "before":
                # Only remove the exact child this invocation created, never a supplied path.
                if before.is_symlink() or before.resolve() != work / "before":
                    raise ValueError("refusing to remove an unexpected baseline target")
                shutil.rmtree(before)
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        result["source_restored"] = manifest.read_bytes() == original_manifest and read_optional(config) == original_config
        result["valid_for_comparison"] = result["source_restored"] and len(result["runs"]) == 4 and all(run["exit_code"] == 0 for run in result["runs"])
        if result["source_restored"]:
            try:
                if work.is_symlink() or work.parent != temp_parent or work.resolve() != work:
                    raise ValueError("refusing to remove an unexpected measurement directory")
                shutil.rmtree(work)
            except (OSError, ValueError) as error:
                result["cleanup_error"] = str(error)
        save(output, result)
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as stream:
                stream.write("\n### Build profile measurement\n\n| Profile | Build | Seconds | Target bytes | Exit |\n|---|---|---:|---:|---:|\n")
                for run in result["runs"]:
                    stream.write(f"| {run['variant']} | {run['temperature']} | {run['wall_seconds']:.3f} | {run['target_bytes']} | {run['exit_code']} |\n")
                stream.write(f"\nSource restored: {result['source_restored']}; valid comparison: {result['valid_for_comparison']}\n")
    if not result["source_restored"]:
        raise RuntimeError("original source bytes were not restored")
    if result.get("cleanup_error"):
        raise RuntimeError(result["cleanup_error"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["detect", "measure"])
    parser.add_argument("--base", default="HEAD^")
    parser.add_argument("--output-dir", default="profile-results", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    base, reasons = changes(root, args.base)
    print(json.dumps({"base": base, "changed": bool(reasons), "reasons": reasons}))
    if args.mode == "detect":
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
                stream.write(f"changed={str(bool(reasons)).lower()}\n")
    elif reasons:
        measure(root, base, args.output_dir.resolve())


if __name__ == "__main__":
    main()
