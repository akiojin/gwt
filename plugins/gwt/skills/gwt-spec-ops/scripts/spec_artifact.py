#!/usr/bin/env python3
"""Manage gwt SPEC artifacts as local files under specs/SPEC-{id}/ directories."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path

VALID_KINDS = {"doc", "contract", "checklist"}

# Maps artifact kind to subdirectory (None = spec root)
KIND_SUBDIR = {
    "doc": None,
    "contract": "contracts",
    "checklist": "checklists",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Manage gwt SPEC artifacts as local files."
    )
    parser.add_argument("--repo", default=".")
    parser.add_argument("--spec", help="SPEC ID (UUID8 string like 'a1b2c3d4')")
    parser.add_argument("--artifact", help="Artifact key like doc:spec.md")
    parser.add_argument("--body-file", help="Path to artifact content file")
    parser.add_argument("--title", help="Title for new SPEC (used with --create)")
    parser.add_argument("--json", action="store_true")

    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--list", action="store_true")
    action.add_argument("--get", action="store_true")
    action.add_argument("--upsert", action="store_true")
    action.add_argument("--create", action="store_true")
    action.add_argument("--close", action="store_true")

    return parser.parse_args()


def find_git_root(start: Path) -> Path:
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=str(start),
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        stderr = proc.stderr.strip() or proc.stdout.strip() or "unknown error"
        raise RuntimeError(f"git rev-parse --show-toplevel: {stderr}")
    return Path(proc.stdout.strip())


def parse_artifact_key(value: str | None) -> tuple[str, str]:
    if not value or ":" not in value:
        raise RuntimeError("artifact key must look like 'doc:spec.md'")
    kind, name = value.split(":", 1)
    if kind not in VALID_KINDS:
        raise RuntimeError(f"unsupported artifact kind: {kind}")
    if not name:
        raise RuntimeError("artifact name must not be empty")
    return kind, name


def normalize_content(raw: str) -> str:
    return raw.rstrip() + "\n"


def spec_dir(git_root: Path, spec_id: str) -> Path:
    return git_root / "specs" / f"SPEC-{spec_id}"


def artifact_path(git_root: Path, spec_id: str, kind: str, name: str) -> Path:
    base = spec_dir(git_root, spec_id)
    subdir = KIND_SUBDIR.get(kind)
    if subdir:
        return base / subdir / name
    return base / name


def ensure_spec_exists(git_root: Path, spec_id: str) -> Path:
    d = spec_dir(git_root, spec_id)
    if not d.is_dir():
        raise RuntimeError(f"SPEC directory not found: {d}")
    return d


def read_metadata(git_root: Path, spec_id: str) -> dict:
    meta_path = spec_dir(git_root, spec_id) / "metadata.json"
    if not meta_path.exists():
        raise RuntimeError(f"metadata.json not found: {meta_path}")
    return json.loads(meta_path.read_text(encoding="utf-8"))


def write_metadata(git_root: Path, spec_id: str, metadata: dict) -> None:
    meta_path = spec_dir(git_root, spec_id) / "metadata.json"
    meta_path.write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def update_timestamp(git_root: Path, spec_id: str) -> None:
    metadata = read_metadata(git_root, spec_id)
    metadata["updated_at"] = datetime.now(timezone.utc).isoformat()
    write_metadata(git_root, spec_id, metadata)


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


# --- Commands ---


def cmd_create(git_root: Path, title: str | None, as_json: bool) -> int:
    if not title:
        print("--title is required with --create", file=sys.stderr)
        return 1

    spec_id = uuid.uuid4().hex[:8]
    d = spec_dir(git_root, spec_id)
    d.mkdir(parents=True, exist_ok=True)

    now = now_iso()
    metadata = {
        "id": spec_id,
        "title": title,
        "status": "open",
        "phase": "draft",
        "created_at": now,
        "updated_at": now,
    }
    write_metadata(git_root, spec_id, metadata)

    if as_json:
        print(json.dumps({"id": spec_id, "path": str(d)}, indent=2))
    else:
        print(spec_id)
    return 0


def cmd_close(git_root: Path, spec_id: str, as_json: bool) -> int:
    ensure_spec_exists(git_root, spec_id)
    metadata = read_metadata(git_root, spec_id)
    metadata["status"] = "closed"
    metadata["updated_at"] = now_iso()
    write_metadata(git_root, spec_id, metadata)

    if as_json:
        print(json.dumps(metadata, ensure_ascii=False, indent=2))
    else:
        print(f"SPEC-{spec_id} closed.")
    return 0


def collect_artifacts(git_root: Path, spec_id: str) -> list[dict]:
    d = ensure_spec_exists(git_root, spec_id)
    artifacts = []

    # Scan doc artifacts (files directly in spec dir, excluding metadata.json)
    for f in sorted(d.iterdir()):
        if f.is_file() and f.name != "metadata.json":
            stat = f.stat()
            artifacts.append({
                "key": f"doc:{f.name}",
                "kind": "doc",
                "name": f.name,
                "path": str(f),
                "modified_at": datetime.fromtimestamp(
                    stat.st_mtime, tz=timezone.utc
                ).isoformat(),
            })

    # Scan contract and checklist subdirectories
    for kind, subdir_name in (("contract", "contracts"), ("checklist", "checklists")):
        subdir = d / subdir_name
        if subdir.is_dir():
            for f in sorted(subdir.iterdir()):
                if f.is_file():
                    stat = f.stat()
                    artifacts.append({
                        "key": f"{kind}:{f.name}",
                        "kind": kind,
                        "name": f.name,
                        "path": str(f),
                        "modified_at": datetime.fromtimestamp(
                            stat.st_mtime, tz=timezone.utc
                        ).isoformat(),
                    })

    return sorted(artifacts, key=lambda a: a["key"])


def cmd_list(git_root: Path, spec_id: str, as_json: bool) -> int:
    artifacts = collect_artifacts(git_root, spec_id)

    if as_json:
        print(json.dumps(artifacts, ensure_ascii=False, indent=2))
        return 0

    if not artifacts:
        print("No artifacts found.")
        return 0

    for a in artifacts:
        print(f"- {a['key']}")
    return 0


def cmd_get(
    git_root: Path, spec_id: str, artifact_key: str | None, as_json: bool
) -> int:
    kind, name = parse_artifact_key(artifact_key)
    ensure_spec_exists(git_root, spec_id)

    fpath = artifact_path(git_root, spec_id, kind, name)
    if not fpath.is_file():
        print(f"Artifact not found: {kind}:{name}", file=sys.stderr)
        return 1

    content = normalize_content(fpath.read_text(encoding="utf-8"))
    stat = fpath.stat()
    modified_at = datetime.fromtimestamp(stat.st_mtime, tz=timezone.utc).isoformat()

    if as_json:
        print(
            json.dumps(
                {
                    "artifact": f"{kind}:{name}",
                    "path": str(fpath),
                    "modified_at": modified_at,
                    "content": content,
                },
                ensure_ascii=False,
                indent=2,
            )
        )
    else:
        sys.stdout.write(content)
    return 0


def cmd_upsert(
    git_root: Path,
    spec_id: str,
    artifact_key: str | None,
    body_file: str | None,
    as_json: bool,
) -> int:
    if not body_file:
        print("--body-file is required with --upsert", file=sys.stderr)
        return 1

    kind, name = parse_artifact_key(artifact_key)
    ensure_spec_exists(git_root, spec_id)

    source = Path(body_file)
    if not source.is_file():
        raise RuntimeError(f"body-file not found: {source}")

    content = normalize_content(source.read_text(encoding="utf-8"))
    fpath = artifact_path(git_root, spec_id, kind, name)
    fpath.parent.mkdir(parents=True, exist_ok=True)
    fpath.write_text(content, encoding="utf-8")

    update_timestamp(git_root, spec_id)

    stat = fpath.stat()
    modified_at = datetime.fromtimestamp(stat.st_mtime, tz=timezone.utc).isoformat()

    if as_json:
        print(
            json.dumps(
                {
                    "artifact": f"{kind}:{name}",
                    "path": str(fpath),
                    "modified_at": modified_at,
                    "content": content,
                },
                ensure_ascii=False,
                indent=2,
            )
        )
    else:
        print(f"Upserted {kind}:{name} -> {fpath}")
    return 0


def main() -> int:
    args = parse_args()
    repo_root = find_git_root(Path(args.repo).resolve())

    if args.create:
        return cmd_create(repo_root, args.title, args.json)

    if not args.spec:
        print("--spec is required", file=sys.stderr)
        return 1

    spec_id = args.spec

    if args.close:
        return cmd_close(repo_root, spec_id, args.json)

    if args.list:
        return cmd_list(repo_root, spec_id, args.json)

    if args.get:
        return cmd_get(repo_root, spec_id, args.artifact, args.json)

    if args.upsert:
        return cmd_upsert(repo_root, spec_id, args.artifact, args.body_file, args.json)

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as err:
        print(str(err), file=sys.stderr)
        raise SystemExit(1)
