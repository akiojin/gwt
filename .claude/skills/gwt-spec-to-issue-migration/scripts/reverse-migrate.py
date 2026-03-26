#!/usr/bin/env python3
"""Reverse-migrate gwt-spec GitHub Issues to local specs/SPEC-{UUID8}/ directories."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import uuid
from dataclasses import asdict, dataclass, field
from pathlib import Path

ARTIFACT_MARKER_RE = re.compile(r"^<!--\s*GWT_SPEC_ARTIFACT:([^:]+):(.+?)\s*-->\s*$")
VALID_KINDS = {"doc", "contract", "checklist"}

# Artifact kind -> subdirectory mapping
KIND_SUBDIR = {
    "contract": "contracts",
    "checklist": "checklists",
}

# doc artifacts that go directly in the SPEC root
DOC_ROOT_FILES = {
    "spec.md", "plan.md", "tasks.md", "research.md",
    "data-model.md", "quickstart.md",
}


@dataclass
class Metadata:
    id: str
    title: str
    status: str
    phase: str
    created_at: str
    updated_at: str


@dataclass
class MigrationEntry:
    issue_number: int
    spec_id: str
    title: str
    status: str  # success | failed | dry-run
    artifact_count: int = 0
    errors: list[str] = field(default_factory=list)


def run(cmd: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
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


def find_git_root(start: Path) -> Path:
    proc = run(["git", "rev-parse", "--show-toplevel"], start)
    return Path(require_success(proc, "git rev-parse --show-toplevel"))


def generate_spec_id() -> str:
    return uuid.uuid4().hex[:8]


def fetch_all_spec_issues(repo_root: Path) -> list[dict]:
    proc = run(
        ["gh", "issue", "list", "--label", "gwt-spec", "--state", "all",
         "--limit", "500", "--json",
         "number,title,body,state,createdAt,updatedAt,labels"],
        repo_root,
    )
    data = require_success(proc, "gh issue list")
    issues = json.loads(data or "[]")
    if not isinstance(issues, list):
        raise RuntimeError("gh issue list: expected JSON array")
    return issues


def fetch_repo_slug(repo_root: Path) -> str:
    proc = run(["gh", "repo", "view", "--json", "nameWithOwner"], repo_root)
    data = json.loads(require_success(proc, "gh repo view"))
    slug = data.get("nameWithOwner")
    if not slug:
        raise RuntimeError("gh repo view: missing nameWithOwner")
    return str(slug)


def fetch_issue_comments(repo_root: Path, repo_slug: str, issue_number: int) -> list[dict]:
    proc = run(
        ["gh", "api", f"repos/{repo_slug}/issues/{issue_number}/comments?per_page=100"],
        repo_root,
    )
    data = require_success(proc, f"gh api issue #{issue_number} comments")
    comments = json.loads(data or "[]")
    if not isinstance(comments, list):
        raise RuntimeError(f"gh api issue #{issue_number} comments: expected JSON array")
    return comments


def extract_content(body: str, kind: str, name: str) -> str:
    lines = body.splitlines()
    if len(lines) >= 2 and lines[1].strip() == f"{kind}:{name}":
        remainder = "\n".join(lines[2:]).lstrip("\n")
        return remainder.rstrip() + "\n"
    return body.rstrip() + "\n"


def parse_artifact_comments(comments: list[dict]) -> list[tuple[str, str, str]]:
    """Parse artifact comments. Returns list of (kind, name, content)."""
    artifacts = []
    for comment in comments:
        body = str(comment.get("body") or "")
        first_line = body.splitlines()[0] if body.splitlines() else ""
        match = ARTIFACT_MARKER_RE.match(first_line.strip())
        if not match:
            continue
        kind, name = match.groups()
        if kind not in VALID_KINDS:
            continue
        content = extract_content(body, kind, name)
        artifacts.append((kind, name, content))
    return artifacts


def detect_phase(body: str) -> str:
    phase_re = re.compile(r"Phase:\s*(\S+)", re.IGNORECASE)
    match = phase_re.search(body)
    if match:
        raw = match.group(1).strip().lower()
        phase_map = {
            "specify": "draft",
            "ready": "ready",
            "planned": "planned",
            "ready for dev": "ready-for-dev",
            "in progress": "in-progress",
            "done": "done",
            "blocked": "blocked",
        }
        return phase_map.get(raw, raw)
    return "draft"


def parse_issue_body_sections(body: str) -> list[tuple[str, str, str]]:
    """Extract sections from Issue body as fallback artifacts.

    Parses `## Spec`, `## Plan`, `## Tasks`, etc. from the Issue body
    and returns them as (kind, name, content) tuples.
    """
    section_map = {
        "Spec": ("doc", "spec.md"),
        "Plan": ("doc", "plan.md"),
        "Tasks": ("doc", "tasks.md"),
        "TDD": ("checklist", "tdd.md"),
        "Research": ("doc", "research.md"),
        "Data Model": ("doc", "data-model.md"),
        "Quickstart": ("doc", "quickstart.md"),
        "Contracts": ("doc", "contracts.md"),
        "Checklists": ("doc", "checklists.md"),
        "Checklist": ("checklist", "acceptance.md"),
        "Acceptance Checklist": ("checklist", "acceptance.md"),
    }

    artifacts = []
    regex = re.compile(r"^##\s+(.+)$", re.MULTILINE)
    matches = list(regex.finditer(body))

    for i, match in enumerate(matches):
        title = match.group(1).strip()
        if title not in section_map:
            continue
        start = match.end()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(body)
        content = body[start:end].strip()
        if not content or content == "_TODO_":
            continue
        kind, name = section_map[title]
        artifacts.append((kind, name, content + "\n"))

    return artifacts


def write_spec_dir(
    specs_dir: Path,
    spec_id: str,
    issue: dict,
    artifacts: list[tuple[str, str, str]],
) -> int:
    spec_dir = specs_dir / f"SPEC-{spec_id}"
    spec_dir.mkdir(parents=True, exist_ok=True)

    # Write metadata.json
    body = str(issue.get("body") or "")
    state = str(issue.get("state") or "OPEN").lower()
    metadata = Metadata(
        id=spec_id,
        title=str(issue.get("title") or "Untitled"),
        status="closed" if state == "closed" else "open",
        phase=detect_phase(body),
        created_at=str(issue.get("createdAt") or ""),
        updated_at=str(issue.get("updatedAt") or ""),
    )
    meta_path = spec_dir / "metadata.json"
    meta_path.write_text(
        json.dumps(asdict(metadata), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    # If no artifact comments found, try extracting from Issue body sections
    if not artifacts:
        artifacts = parse_issue_body_sections(body)

    # Write artifact files
    written = 0
    for kind, name, content in artifacts:
        if kind == "doc" and name in DOC_ROOT_FILES:
            file_path = spec_dir / name
        elif kind in KIND_SUBDIR:
            subdir = spec_dir / KIND_SUBDIR[kind]
            subdir.mkdir(exist_ok=True)
            file_path = subdir / name
        else:
            file_path = spec_dir / name

        file_path.write_text(content, encoding="utf-8")
        written += 1

    return written


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Reverse-migrate gwt-spec GitHub Issues to local specs/ directories"
    )
    parser.add_argument("--repo", default=".", help="Path to repository")
    parser.add_argument("--dry-run", action="store_true", help="Show what would be done")
    parser.add_argument(
        "--specs-dir", default="",
        help="Output directory for specs (default: <repo>/specs)"
    )
    parser.add_argument(
        "--report", default="reverse-migration-report.json",
        help="Migration report output file"
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo_root = find_git_root(Path(args.repo).resolve())
    specs_dir = Path(args.specs_dir) if args.specs_dir else repo_root / "specs"

    print(f"Repository: {repo_root}")
    print(f"Output: {specs_dir}")
    print(f"Dry run: {args.dry_run}")
    print()

    # Fetch all gwt-spec issues
    print("Fetching gwt-spec issues...")
    issues = fetch_all_spec_issues(repo_root)
    print(f"Found {len(issues)} gwt-spec issues")

    if not issues:
        print("Nothing to migrate.")
        return 0

    # Fetch repo slug for API calls
    repo_slug = fetch_repo_slug(repo_root)
    print(f"Repository: {repo_slug}")
    print()

    report: list[dict] = []
    success_count = 0
    fail_count = 0

    for i, issue in enumerate(issues, 1):
        number = issue["number"]
        title = issue.get("title", "Untitled")
        spec_id = generate_spec_id()

        print(f"[{i}/{len(issues)}] Issue #{number}: {title}")

        if args.dry_run:
            print(f"  [dry-run] Would create SPEC-{spec_id}/")
            report.append(asdict(MigrationEntry(
                issue_number=number, spec_id=spec_id,
                title=title, status="dry-run",
            )))
            continue

        # Fetch comments and parse artifacts
        errors: list[str] = []
        try:
            comments = fetch_issue_comments(repo_root, repo_slug, number)
            artifacts = parse_artifact_comments(comments)
        except RuntimeError as err:
            print(f"  ERROR: {err}")
            report.append(asdict(MigrationEntry(
                issue_number=number, spec_id=spec_id,
                title=title, status="failed", errors=[str(err)],
            )))
            fail_count += 1
            continue

        # Write local files
        try:
            artifact_count = write_spec_dir(specs_dir, spec_id, issue, artifacts)
            print(f"  Created SPEC-{spec_id}/ ({artifact_count} artifacts)")
            report.append(asdict(MigrationEntry(
                issue_number=number, spec_id=spec_id,
                title=title, status="success",
                artifact_count=artifact_count,
            )))
            success_count += 1
        except OSError as err:
            print(f"  ERROR writing files: {err}")
            errors.append(str(err))
            report.append(asdict(MigrationEntry(
                issue_number=number, spec_id=spec_id,
                title=title, status="failed", errors=errors,
            )))
            fail_count += 1

    # Write report
    report_path = Path(args.report)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    print()
    print(f"Migration complete: {success_count} succeeded, {fail_count} failed out of {len(issues)} total")
    print(f"Report: {report_path}")

    return 1 if fail_count > 0 else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as err:
        print(str(err), file=sys.stderr)
        raise SystemExit(1)
