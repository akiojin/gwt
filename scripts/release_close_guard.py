#!/usr/bin/env python3
"""Reopen Issues that a Release PR merge closed by accident (Issue #3545 AC-3).

`main` is the default branch, so any closing keyword that reaches it — in the
Release PR body or in a commit message carried by the merge — closes the
referenced Issue even when acceptance criteria are still open. This guard runs
from `release.yml` right after the merge, lists Issues closed at or after the
merge time, keeps the ones whose closer is the Release PR itself or a commit,
and reopens them with a marker comment. Issues closed through the API (Issue
Monitor settlement, humans) have no closer and are left alone.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Callable, Sequence

CommandRunner = Callable[[Sequence[str]], str]

GUARD_MARKER_PREFIX = "<!-- gwt-release-close-guard v1"

CLOSED_ISSUES_QUERY = """
query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    issues(states: CLOSED, first: 50, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number
        title
        closedAt
        timelineItems(last: 1, itemTypes: [CLOSED_EVENT]) {
          nodes {
            ... on ClosedEvent {
              createdAt
              closer {
                __typename
                ... on PullRequest { number }
                ... on Commit { oid }
              }
            }
          }
        }
      }
    }
  }
}
""".strip()


class CommandError(RuntimeError):
    """A `gh` invocation failed; the message carries stderr."""


@dataclass
class ReleaseClosure:
    number: int
    title: str
    closed_at: str
    closer: str


@dataclass
class GuardReport:
    repo: str
    merge_sha: str
    release_pr: int | None = None
    merged_at: str | None = None
    dry_run: bool = False
    detected: list[ReleaseClosure] = field(default_factory=list)
    reopened: list[int] = field(default_factory=list)
    failed: list[tuple[int, str]] = field(default_factory=list)


def run_command(args: Sequence[str]) -> str:
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        raise CommandError((result.stderr or result.stdout or "").strip() or f"exit {result.returncode}")
    return result.stdout


def parse_timestamp(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)


def resolve_repo_slug(runner: CommandRunner) -> str:
    return runner(["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"]).strip()


def find_release_pr(merge_sha: str, repo_slug: str, runner: CommandRunner) -> tuple[int, str] | None:
    """Return `(number, merged_at)` of the merged PR whose merge commit is `merge_sha`."""
    payload = json.loads(runner(["gh", "api", f"repos/{repo_slug}/commits/{merge_sha}/pulls"]) or "[]")
    for pull in payload:
        if pull.get("merge_commit_sha") == merge_sha and pull.get("merged_at"):
            return int(pull["number"]), str(pull["merged_at"])
    return None


def fetch_closed_issues(repo_slug: str, runner: CommandRunner) -> list[dict]:
    owner, name = repo_slug.split("/", 1)
    payload = json.loads(
        runner(
            [
                "gh",
                "api",
                "graphql",
                "-f",
                f"query={CLOSED_ISSUES_QUERY}",
                "-F",
                f"owner={owner}",
                "-F",
                f"name={name}",
            ]
        )
        or "{}"
    )
    return payload.get("data", {}).get("repository", {}).get("issues", {}).get("nodes", []) or []


def describe_closer(closer: dict | None) -> str | None:
    if not closer:
        return None
    kind = closer.get("__typename")
    if kind == "PullRequest":
        return f"PR #{closer.get('number')}"
    if kind == "Commit":
        return f"commit {closer.get('oid')}"
    return None


def detect_release_closures(issues: Sequence[dict], release_pr: int, merged_at: str) -> list[ReleaseClosure]:
    """Keep Issues the release merge closed: closer is the Release PR or a commit.

    API closures (Issue Monitor settlement, `gh issue close`, the web UI) carry
    no closer and are intentional. A different PR closing an Issue after the
    merge is that PR's decision, not the release's.
    """
    merged = parse_timestamp(merged_at)
    detected: list[ReleaseClosure] = []
    for issue in issues:
        closed_at = issue.get("closedAt")
        if not closed_at or parse_timestamp(closed_at) < merged:
            continue
        events = issue.get("timelineItems", {}).get("nodes", []) or []
        closer = events[-1].get("closer") if events else None
        if not closer:
            continue
        kind = closer.get("__typename")
        if kind == "PullRequest" and int(closer.get("number", 0)) != release_pr:
            continue
        label = describe_closer(closer)
        if label is None:
            continue
        detected.append(
            ReleaseClosure(
                number=int(issue["number"]),
                title=str(issue.get("title", "")),
                closed_at=str(closed_at),
                closer=label,
            )
        )
    detected.sort(key=lambda item: item.number)
    return detected


def render_reopen_comment(issue_number: int, release_pr: int, merge_sha: str, closer: str) -> str:
    return (
        f"{GUARD_MARKER_PREFIX} pr={release_pr} sha={merge_sha} -->\n\n"
        f"Release PR #{release_pr}（merge commit `{merge_sha}`）の main への merge で "
        f"Issue #{issue_number} が閉じられました（closer: {closer}）。"
        "Release PR は Issue を close しない運用（Issue #3545）のため、"
        "gwt release close guard が reopen します。\n\n"
        "受け入れ基準の決着は work ブランチの develop merge 時に Issue Monitor が行います（Issue #3917）。"
        "本当に完了している場合はそちらの settlement コメントを確認のうえ手動で close してください。\n"
    )


def run_guard(merge_sha: str, repo_slug: str | None, dry_run: bool, runner: CommandRunner = run_command) -> GuardReport:
    repo = repo_slug or resolve_repo_slug(runner)
    report = GuardReport(repo=repo, merge_sha=merge_sha, dry_run=dry_run)

    release = find_release_pr(merge_sha, repo, runner)
    if release is None:
        return report
    report.release_pr, report.merged_at = release

    issues = fetch_closed_issues(repo, runner)
    report.detected = detect_release_closures(issues, report.release_pr, report.merged_at)
    if dry_run:
        return report

    for closure in report.detected:
        comment = render_reopen_comment(closure.number, report.release_pr, merge_sha, closure.closer)
        try:
            runner(["gh", "issue", "reopen", str(closure.number), "--repo", repo, "--comment", comment])
        except CommandError as error:
            report.failed.append((closure.number, str(error)))
            continue
        report.reopened.append(closure.number)
    return report


def exit_code(report: GuardReport) -> int:
    return 1 if report.failed else 0


def render_text(report: GuardReport) -> str:
    lines = [f"Repo: {report.repo}", f"Merge commit: {report.merge_sha}"]
    if report.release_pr is None:
        lines.append("Release PR: none found for this commit; nothing to guard.")
        return "\n".join(lines)
    lines.append(f"Release PR: #{report.release_pr} (merged at {report.merged_at})")
    mode = "dry-run" if report.dry_run else "reopen"
    lines.append(f"Mode: {mode}")
    lines.append("")
    if not report.detected:
        lines.append("Issues closed by the release merge: none")
        return "\n".join(lines)
    lines.append("Issues closed by the release merge:")
    for closure in report.detected:
        status = "detected"
        if closure.number in report.reopened:
            status = "reopened"
        elif any(number == closure.number for number, _ in report.failed):
            status = "reopen FAILED"
        lines.append(f"- #{closure.number} {closure.title} (closer: {closure.closer}, {closure.closed_at}) — {status}")
    for number, error in report.failed:
        lines.append(f"  #{number}: {error}")
    return "\n".join(lines)


def render_annotations(report: GuardReport) -> str:
    """GitHub Actions workflow commands so the run surfaces the outcome."""
    lines: list[str] = []
    for closure in report.detected:
        if closure.number in report.reopened:
            lines.append(f"::warning::Release PR #{report.release_pr} closed Issue #{closure.number} ({closure.closer}); reopened by the release close guard.")
        elif report.dry_run:
            lines.append(f"::warning::Release PR #{report.release_pr} closed Issue #{closure.number} ({closure.closer}); dry-run, not reopened.")
    for number, error in report.failed:
        lines.append(f"::warning::Release PR #{report.release_pr} closed Issue #{number} and reopening failed: {error}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Reopen Issues closed by a Release PR merge.")
    parser.add_argument("--merge-sha", required=True, help="Merge commit of the Release PR on main.")
    parser.add_argument("--repo", dest="repo_slug", default=None, help="GitHub repo slug (owner/name).")
    parser.add_argument("--dry-run", action="store_true", help="Report without reopening.")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        report = run_guard(args.merge_sha, args.repo_slug, args.dry_run)
    except CommandError as error:
        print(f"release close guard could not query GitHub: {error}", file=sys.stderr)
        return 1

    if args.format == "json":
        payload = {
            "repo": report.repo,
            "merge_sha": report.merge_sha,
            "release_pr": report.release_pr,
            "merged_at": report.merged_at,
            "dry_run": report.dry_run,
            "detected": [closure.__dict__ for closure in report.detected],
            "reopened": report.reopened,
            "failed": [{"number": number, "error": error} for number, error in report.failed],
        }
        print(json.dumps(payload, ensure_ascii=False, indent=2))
    else:
        print(render_text(report))
    annotations = render_annotations(report)
    if annotations:
        print(annotations)
    return exit_code(report)


if __name__ == "__main__":
    raise SystemExit(main())
