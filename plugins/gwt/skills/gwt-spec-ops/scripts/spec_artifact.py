#!/usr/bin/env python3
"""Manage gwt SPEC artifact comments on GitHub Issues."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ARTIFACT_MARKER_RE = re.compile(r"^<!--\s*GWT_SPEC_ARTIFACT:([^:]+):(.+?)\s*-->\s*$")
VALID_KINDS = {"doc", "contract", "checklist"}
SECONDARY_RATE_LIMIT_MARKERS = (
    "submitted too quickly",
    "secondary rate limit",
    "retry-after",
)
RETRY_SLEEP_SECONDS = (5, 15, 30, 60)


@dataclass
class ArtifactComment:
    kind: str
    name: str
    comment_id: int
    body: str
    content: str
    created_at: str
    updated_at: str
    author: str

    @property
    def key(self) -> str:
        return f"{self.kind}:{self.name}"


def run(cmd: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd),
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
    )


def require_success(proc: subprocess.CompletedProcess[str], context: str) -> str:
    if proc.returncode != 0:
        stderr = proc.stderr.strip() or proc.stdout.strip() or "unknown error"
        raise RuntimeError(f"{context}: {stderr}")
    return proc.stdout.strip()


def should_retry(proc: subprocess.CompletedProcess[str]) -> bool:
    message = f"{proc.stderr}\n{proc.stdout}".lower()
    return any(marker in message for marker in SECONDARY_RATE_LIMIT_MARKERS)


def run_with_retry(cmd: list[str], cwd: Path, context: str) -> subprocess.CompletedProcess[str]:
    last_proc: subprocess.CompletedProcess[str] | None = None
    for attempt, sleep_seconds in enumerate((0, *RETRY_SLEEP_SECONDS), start=1):
        if sleep_seconds:
            time.sleep(sleep_seconds)
        proc = run(cmd, cwd)
        last_proc = proc
        if proc.returncode == 0 or not should_retry(proc):
            return proc
    assert last_proc is not None
    return last_proc


def run_gh_api_with_json(
    repo_root: Path,
    endpoint: str,
    method: str,
    payload: dict[str, Any],
    context: str,
) -> dict[str, Any]:
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False) as tmp:
        json.dump(payload, tmp, ensure_ascii=False)
        tmp_path = Path(tmp.name)
    try:
        proc = run_with_retry(
            ["gh", "api", endpoint, "--method", method, "--input", str(tmp_path)],
            repo_root,
            context,
        )
        data = require_success(proc, context)
        return json.loads(data or "{}")
    finally:
        tmp_path.unlink(missing_ok=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=".")
    parser.add_argument("--issue", required=True)
    parser.add_argument("--artifact", help="Artifact key like doc:spec.md")
    parser.add_argument("--body-file", help="Path to artifact markdown content")
    parser.add_argument("--json", action="store_true")

    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--list", action="store_true")
    action.add_argument("--get", action="store_true")
    action.add_argument("--upsert", action="store_true")

    return parser.parse_args()


def find_git_root(start: Path) -> Path:
    proc = run(["git", "rev-parse", "--show-toplevel"], start)
    root = require_success(proc, "git rev-parse --show-toplevel")
    return Path(root)


def ensure_gh_auth(repo_root: Path) -> None:
    proc = run(["gh", "auth", "status"], repo_root)
    require_success(proc, "gh auth status")


def fetch_repo_slug(repo_root: Path) -> str:
    proc = run(["gh", "repo", "view", "--json", "nameWithOwner"], repo_root)
    data = json.loads(require_success(proc, "gh repo view"))
    slug = data.get("nameWithOwner")
    if not slug:
        raise RuntimeError("gh repo view: missing nameWithOwner")
    return str(slug)


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


def build_comment_body(kind: str, name: str, content: str) -> str:
    normalized = normalize_content(content)
    return (
        f"<!-- GWT_SPEC_ARTIFACT:{kind}:{name} -->\n"
        f"{kind}:{name}\n\n"
        f"{normalized}"
    )


def extract_content(body: str, kind: str, name: str) -> str:
    lines = body.splitlines()
    if len(lines) >= 2 and lines[1].strip() == f"{kind}:{name}":
        remainder = "\n".join(lines[2:]).lstrip("\n")
        return normalize_content(remainder)
    return normalize_content(body)


def parse_artifact_comment(comment: dict[str, Any]) -> ArtifactComment | None:
    body = str(comment.get("body") or "")
    first_line = body.splitlines()[0] if body.splitlines() else ""
    match = ARTIFACT_MARKER_RE.match(first_line.strip())
    if not match:
        return None

    kind, name = match.groups()
    if kind not in VALID_KINDS:
        return None

    content = extract_content(body, kind, name)
    return ArtifactComment(
        kind=kind,
        name=name,
        comment_id=int(comment["id"]),
        body=body,
        content=content,
        created_at=str(comment.get("created_at") or ""),
        updated_at=str(comment.get("updated_at") or ""),
        author=str((comment.get("user") or {}).get("login") or "unknown"),
    )


def fetch_issue_comments(repo_root: Path, repo_slug: str, issue_number: str) -> list[dict[str, Any]]:
    proc = run(
        ["gh", "api", f"repos/{repo_slug}/issues/{issue_number}/comments?per_page=100"],
        repo_root,
    )
    payload = require_success(proc, "gh api issue comments")
    comments = json.loads(payload or "[]")
    if not isinstance(comments, list):
        raise RuntimeError("gh api issue comments: expected JSON array")
    return comments


def collect_artifacts(repo_root: Path, issue_number: str) -> tuple[str, list[ArtifactComment]]:
    repo_slug = fetch_repo_slug(repo_root)
    comments = fetch_issue_comments(repo_root, repo_slug, issue_number)
    artifacts = [parsed for comment in comments if (parsed := parse_artifact_comment(comment))]
    return repo_slug, sorted(artifacts, key=lambda item: item.key)


def upsert_artifact(
    repo_root: Path,
    repo_slug: str,
    issue_number: str,
    kind: str,
    name: str,
    content: str,
) -> ArtifactComment:
    _, artifacts = collect_artifacts(repo_root, issue_number)
    existing = next((artifact for artifact in artifacts if artifact.key == f"{kind}:{name}"), None)
    body = build_comment_body(kind, name, content)

    if existing:
        data = run_gh_api_with_json(
            repo_root,
            f"repos/{repo_slug}/issues/comments/{existing.comment_id}",
            "PATCH",
            {"body": body},
            "gh api patch issue comment",
        )
    else:
        data = run_gh_api_with_json(
            repo_root,
            f"repos/{repo_slug}/issues/{issue_number}/comments",
            "POST",
            {"body": body},
            "gh api create issue comment",
        )

    parsed = parse_artifact_comment(data)
    if not parsed:
        raise RuntimeError("failed to parse created artifact comment")
    return parsed


def print_artifact_list(artifacts: list[ArtifactComment], as_json: bool) -> None:
    if as_json:
        payload = [
            {
                "artifact": artifact.key,
                "comment_id": artifact.comment_id,
                "author": artifact.author,
                "created_at": artifact.created_at,
                "updated_at": artifact.updated_at,
            }
            for artifact in artifacts
        ]
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        return

    if not artifacts:
        print("No artifact comments found.")
        return

    for artifact in artifacts:
        print(f"- {artifact.key} (comment {artifact.comment_id}, author={artifact.author})")


def print_single_artifact(artifact: ArtifactComment, as_json: bool) -> None:
    if as_json:
        print(
            json.dumps(
                {
                    "artifact": artifact.key,
                    "comment_id": artifact.comment_id,
                    "author": artifact.author,
                    "created_at": artifact.created_at,
                    "updated_at": artifact.updated_at,
                    "content": artifact.content,
                },
                ensure_ascii=False,
                indent=2,
            )
        )
        return

    sys.stdout.write(artifact.content)


def main() -> int:
    args = parse_args()
    repo_root = find_git_root(Path(args.repo).resolve())
    ensure_gh_auth(repo_root)

    issue_number = str(args.issue)
    repo_slug, artifacts = collect_artifacts(repo_root, issue_number)

    if args.list:
        print_artifact_list(artifacts, args.json)
        return 0

    kind, name = parse_artifact_key(args.artifact)

    if args.get:
        artifact = next((item for item in artifacts if item.key == f"{kind}:{name}"), None)
        if not artifact:
            print(f"Artifact not found: {kind}:{name}", file=sys.stderr)
            return 1
        print_single_artifact(artifact, args.json)
        return 0

    if not args.body_file:
        print("--body-file is required with --upsert", file=sys.stderr)
        return 1

    content = Path(args.body_file).read_text(encoding="utf-8")
    artifact = upsert_artifact(repo_root, repo_slug, issue_number, kind, name, content)
    if args.json:
        print_single_artifact(artifact, True)
    else:
        print(f"Upserted {artifact.key} on issue #{issue_number} (comment {artifact.comment_id}).")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as err:
        print(str(err), file=sys.stderr)
        raise SystemExit(1)
