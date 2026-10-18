"""Preserve Cargo source freshness only for content-verified CI inputs.

Run restore before coverage and capture after a successful coverage build.
Cache llvm-cov-target and coverage-cache-source.json together; never cache the
coverage-cache-inputs.json snapshot from the current run. Git metadata is never
retimed. COVERAGE_CACHE_RESTORED must contain a successful native cache matched
key. This helper assumes CI is the only writer of the checked-out source.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time

TARGET = "target/llvm-cov-target"
SIDECAR = "target/coverage-cache-source.json"
INPUTS = "target/coverage-cache-inputs.json"
ERRORS = (OSError, ValueError, KeyError, TypeError, AttributeError, NotImplementedError, subprocess.SubprocessError)


def git(root, *args):
    return subprocess.check_output(["git", "--no-optional-locks", "-C", str(root), *args], stderr=subprocess.PIPE)


def linked(info):
    return stat.S_ISLNK(info.st_mode) or bool(getattr(info, "st_file_attributes", 0) & 0x400)


def bounds(root):
    if not root.is_absolute() or root != root.resolve() or Path(git(root, "rev-parse", "--show-toplevel").decode().strip()).resolve() != root:
        raise ValueError("cwd must be the resolved repository root")
    for path in (root, root / "target", root / TARGET):
        if path.resolve() != path or not path.is_relative_to(root):
            raise ValueError("cache directory escapes repository")
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue
        if linked(info) or not stat.S_ISDIR(info.st_mode):
            raise ValueError("cache boundary is a symlink or non-directory")
    if git(root, "ls-files", "-z", "--", TARGET, SIDECAR, INPUTS):
        raise ValueError("cache paths must not contain tracked source")


def source_path(root, name, directory=False):
    if name == "" and directory:
        return root
    if (not isinstance(name, str) or not name or "\\" in name or ":" in name
            or any(p in ("", ".", "..") or p.lower() == ".git" for p in name.split("/"))):
        raise ValueError("invalid source metadata path")
    path = root.joinpath(*name.split("/"))
    if not path.is_relative_to(root):
        raise ValueError("source metadata escapes repository")
    return path


def ancestors(name):
    parent = Path(name).parent
    while True:
        yield "" if parent == Path(".") else parent.as_posix()
        if parent == Path("."):
            return
        parent = parent.parent


def snapshot(root):
    head = git(root, "rev-parse", "HEAD").decode().strip()
    clean = not git(root, "status", "--porcelain=v1", "-z", "--untracked-files=normal")
    embedded = ("--", ".claude/skills", ".claude/commands")
    clean = clean and not git(root, "ls-files", "--others", "--exclude-standard", "-z", *embedded)
    clean = clean and not git(root, "ls-files", "--others", "--ignored", "--exclude-standard", "-z", *embedded)
    files, directories = {}, {}
    for record in git(root, "ls-files", "--stage", "-z").split(b"\0"):
        if not record:
            continue
        header, raw_name = record.split(b"\t", 1)
        mode, _, stage = header.decode().split()
        name = raw_name.decode("utf-8", errors="strict")
        if mode not in ("100644", "100755") or stage != "0":
            raise ValueError("symlink, submodule, or unresolved index")
        for parent in ancestors(name):
            info = source_path(root, parent, directory=True).lstat()
            if linked(info) or not stat.S_ISDIR(info.st_mode):
                raise ValueError("symlink/non-directory source ancestor")
            directories[parent] = info.st_mtime_ns
        path = source_path(root, name)
        before = path.lstat()
        if linked(before) or not stat.S_ISREG(before.st_mode):
            raise ValueError("symlink/non-regular tracked input")
        content = path.read_bytes()
        after = path.lstat()
        if (before.st_mtime_ns, before.st_ctime_ns, before.st_size) != (after.st_mtime_ns, after.st_ctime_ns, after.st_size):
            raise ValueError("source changed while hashing")
        files[name] = {"sha256": hashlib.sha256(content).hexdigest(), "size": len(content), "mode": mode,
                       "permissions": stat.S_IMODE(before.st_mode), "mtime_ns": before.st_mtime_ns}
    clean = clean and git(root, "rev-parse", "HEAD").decode().strip() == head
    clean = clean and not git(root, "status", "--porcelain=v1", "-z", "--untracked-files=normal")
    return {"schema": 1, "head": head, "clean": clean, "captured_ns": time.time_ns(), "files": files, "directories": directories}


def envelope(data):
    return {"payload": data, "sha256": hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()}


def load(root, path):
    if linked(path.lstat()) or not path.is_file():
        raise ValueError("metadata is not a regular file")
    value = json.loads(path.read_text(encoding="utf-8"))
    data = value["payload"]
    if value != envelope(data) or type(data["schema"]) is not int or data["schema"] != 1 or data["clean"] is not True or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", data["head"]):
        raise ValueError("unverified source metadata")
    captured = data["captured_ns"]
    if type(captured) is not int or not 0 < captured <= time.time_ns():
        raise ValueError("invalid capture timestamp")
    for name, entry in data["files"].items():
        source_path(root, name)
        if (not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]) or entry["mode"] not in ("100644", "100755")
                or type(entry["size"]) is not int or entry["size"] < 0 or type(entry["permissions"]) is not int):
            raise ValueError("invalid content attestation")
    for name, stamp in [(n, e["mtime_ns"]) for n, e in data["files"].items()] + list(data["directories"].items()):
        source_path(root, name, directory=name in data["directories"])
        if type(stamp) is not int or not 0 < stamp <= captured:
            raise ValueError("unverifiable source timestamp")
    return data


def write(root, path, data):
    bounds(root)
    path.parent.mkdir(exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        json.dump(envelope(data), output, sort_keys=True)
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def identity(entry):
    return {k: v for k, v in entry.items() if k != "mtime_ns"}


def restore(root):
    bounds(root)
    try:
        if not os.environ.get("COVERAGE_CACHE_RESTORED", "").strip():
            raise ValueError("native cache restore did not complete")
        old, current = load(root, root / SIDECAR), snapshot(root)
        if not current["clean"]:
            raise ValueError("source is not clean")
        dirty, plan = set(), []
        for name in set(old["files"]) | set(current["files"]):
            previous, present = old["files"].get(name), current["files"].get(name)
            if previous and present and identity(previous) == identity(present):
                plan.append((source_path(root, name), previous["mtime_ns"]))
            else:
                dirty.update(ancestors(name))
        plan += [(source_path(root, name, directory=True), old["directories"][name]) for name in
                 sorted(set(old["directories"]) & set(current["directories"]), key=lambda n: -len(n)) if name not in dirty]
        for path, stamp in plan:
            before = path.lstat()
            if linked(before):
                raise ValueError("symlink appeared while restoring")
            options = {"follow_symlinks": False} if os.utime in os.supports_follow_symlinks else {}
            os.utime(path, ns=(before.st_atime_ns, stamp), **options)
            if path.lstat().st_mtime_ns != stamp:
                raise ValueError("filesystem cannot retain exact timestamp")
        print("coverage-cache: restored content-verified source timestamps")
    except ERRORS as error:
        bounds(root)  # Revalidate the absolute boundary immediately before deletion.
        target = root / TARGET
        if target.exists():
            shutil.rmtree(target)
        (root / SIDECAR).unlink(missing_ok=True)
        print(f"coverage-cache: cold ({error})")
    try:
        beginning = snapshot(root)
    except ERRORS:
        beginning = {"schema": 1, "clean": False}
    write(root, root / INPUTS, beginning)


def capture(root):
    bounds(root)
    cacheable = False
    try:
        beginning, ending = load(root, root / INPUTS), snapshot(root)
        if (not ending["clean"] or beginning["head"] != ending["head"]
                or {n: identity(e) for n, e in beginning["files"].items()} != {n: identity(e) for n, e in ending["files"].items()}
                or not (root / TARGET).is_dir()):
            raise ValueError("build inputs changed or artifacts are missing")
        # Every run starts with new profiles. Keep only reusable build artifacts
        # in the native cache, after checking it cannot contain tracked source.
        bounds(root)
        target = root / TARGET
        for profile in target.rglob("*"):
            if profile.suffix not in (".profraw", ".profdata"):
                continue
            if (linked(profile.lstat()) or not profile.is_file()
                    or not profile.resolve().is_relative_to(target)):
                raise ValueError("profile is outside the instrumented build")
            profile.unlink()
        write(root, root / SIDECAR, ending)
        cacheable = True
    except ERRORS as error:
        (root / SIDECAR).unlink(missing_ok=True)
        print(f"coverage-cache: not cacheable ({error})")
    output = f"cacheable={str(cacheable).lower()}\n"
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as destination:
            destination.write(output)
    else:
        print(output, end="")


if __name__ == "__main__":
    try:
        if sys.argv[1:] not in (["restore"], ["capture"]):
            raise ValueError("usage: ci_coverage_cache.py restore|capture")
        (restore if sys.argv[1] == "restore" else capture)(Path.cwd().absolute())
    except ERRORS as error:
        print(f"coverage-cache: unsafe boundary or unavailable operation: {error}", file=sys.stderr)
        sys.exit(1)
