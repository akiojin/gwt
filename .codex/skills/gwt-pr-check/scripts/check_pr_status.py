#!/usr/bin/env python3
"""Check GitHub PR status for the current branch.

This script mirrors the gwt-pr-check skill rules:
- detect unmerged PRs first
- when all PRs are merged, prefer origin/<head>..HEAD fallback if the merge
  commit is missing or not an ancestor of HEAD
- emit a short human-readable summary by default
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class Result:
    status: str
    action: str
    summary: str
    details: list[str]
    dirty: bool
    head: str
    base: str
    reason: str | None = None
    pr_number: int | None = None
    pr_url: str | None = None
    new_commits: int | None = None
    fallback_used: bool = False


def run(cmd: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd),
        text=True,
        capture_output=True,
        check=False,
        encoding="utf-8",
    )


def require_success(proc: subprocess.CompletedProcess[str], context: str) -> str:
    if proc.returncode != 0:
        stderr = proc.stderr.strip() or proc.stdout.strip() or "unknown error"
        raise RuntimeError(f"{context}: {stderr}")
    return proc.stdout.strip()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=".")
    parser.add_argument("--base", default="develop")
    parser.add_argument("--head")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--lang", choices=["en", "ja"], default="en")
    return parser.parse_args()


def list_prs(repo: Path, head: str) -> list[dict[str, Any]]:
    proc = run(
        [
            "gh",
            "pr",
            "list",
            "--head",
            head,
            "--state",
            "all",
            "--json",
            "number,state,mergedAt,updatedAt,url,title,mergeCommit,baseRefName,headRefName",
        ],
        repo,
    )
    return json.loads(require_success(proc, "gh pr list"))


def git_count(repo: Path, revspec: str) -> int | None:
    proc = run(["git", "rev-list", "--count", revspec], repo)
    if proc.returncode != 0:
        return None
    try:
        return int(proc.stdout.strip())
    except ValueError:
        return None


def is_ancestor(repo: Path, ancestor: str) -> bool:
    proc = run(["git", "merge-base", "--is-ancestor", ancestor, "HEAD"], repo)
    return proc.returncode == 0


def latest_by(items: list[dict[str, Any]], key: str) -> dict[str, Any] | None:
    present = [item for item in items if item.get(key)]
    if not present:
        return None
    return sorted(present, key=lambda item: item.get(key) or "")[-1]


def format_result(result: Result, lang: str) -> str:
    lines: list[str]
    if lang == "ja":
        if result.status == "NO_PR":
            lines = [
                f">> CREATE PR — `{result.head}` -> `{result.base}` の PR は存在しません。"
            ]
        elif result.status == "UNMERGED_PR_EXISTS":
            lines = [
                f"> PUSH ONLY — `{result.head}` の未マージ PR があります。",
                f"   PR: #{result.pr_number} {result.pr_url}",
            ]
        elif result.status == "ALL_MERGED_WITH_NEW_COMMITS":
            lines = [
                f">> CREATE PR — 最終マージ後に {result.new_commits} 件の新しい commit があります (#{result.pr_number})。",
                f"   head: {result.head} -> base: {result.base}",
            ]
        elif result.status == "ALL_MERGED_NO_NEW_COMMITS":
            lines = [
                f"-- NO ACTION — すべての PR はマージ済みで、`{result.head}` に新しい commit はありません。"
            ]
        else:
            lines = [
                "!! MANUAL CHECK — PR 状態を判定できませんでした。",
                f"   Reason: {result.reason}",
                f"   head: {result.head} -> base: {result.base}",
            ]
    else:
        lines = [result.summary, *result.details]

    if result.dirty:
        lines.append("   (!) Worktree has uncommitted changes.")
    return "\n".join(lines)


def build_result(repo: Path, base: str, head: str, dirty: bool) -> Result:
    run(["git", "fetch", "origin"], repo)
    prs = list_prs(repo, head)

    if not prs:
        return Result(
            status="NO_PR",
            action="CREATE_PR",
            summary=f">> CREATE PR — No PR exists for `{head}` -> `{base}`.",
            details=[],
            dirty=dirty,
            head=head,
            base=base,
            reason="No PR found for head branch",
        )

    unmerged = [pr for pr in prs if pr.get("mergedAt") is None]
    if unmerged:
        pr = latest_by(unmerged, "updatedAt") or unmerged[0]
        return Result(
            status="UNMERGED_PR_EXISTS",
            action="PUSH_ONLY",
            summary=f"> PUSH ONLY — Unmerged PR open for `{head}`.",
            details=[f"   PR: #{pr['number']} {pr['url']}"],
            dirty=dirty,
            head=head,
            base=base,
            pr_number=pr["number"],
            pr_url=pr["url"],
            reason="At least one PR for the head branch is not merged",
        )

    latest_merged = latest_by(prs, "mergedAt")
    if latest_merged is None:
        return Result(
            status="CHECK_FAILED",
            action="MANUAL_CHECK",
            summary="!! MANUAL CHECK — Could not determine PR status.",
            details=[
                "   Reason: Could not determine latest merged PR",
                f"   head: {head} -> base: {base}",
            ],
            dirty=dirty,
            head=head,
            base=base,
            reason="Could not determine latest merged PR",
        )

    merge_commit = ((latest_merged.get("mergeCommit") or {}) or {}).get("oid")
    if merge_commit and is_ancestor(repo, merge_commit):
        post_merge_commits = git_count(repo, f"{merge_commit}..HEAD")
        if post_merge_commits is None:
            return Result(
                status="CHECK_FAILED",
                action="MANUAL_CHECK",
                summary="!! MANUAL CHECK — Could not determine PR status.",
                details=[
                    "   Reason: Failed to count commits after merge commit",
                    f"   head: {head} -> base: {base}",
                ],
                dirty=dirty,
                head=head,
                base=base,
                reason="Failed to count commits after merge commit",
            )
        if post_merge_commits > 0:
            return Result(
                status="ALL_MERGED_WITH_NEW_COMMITS",
                action="CREATE_PR",
                summary=f">> CREATE PR — {post_merge_commits} new commit(s) after last merge (#{latest_merged['number']}).",
                details=[f"   head: {head} -> base: {base}"],
                dirty=dirty,
                head=head,
                base=base,
                pr_number=latest_merged["number"],
                new_commits=post_merge_commits,
            )
        return Result(
            status="ALL_MERGED_NO_NEW_COMMITS",
            action="NO_ACTION",
            summary=f"-- NO ACTION — All PRs merged, no new commits on `{head}`.",
            details=[],
            dirty=dirty,
            head=head,
            base=base,
            pr_number=latest_merged["number"],
            new_commits=0,
        )

    upstream_commits = git_count(repo, f"origin/{head}..HEAD")
    if upstream_commits is not None:
        if upstream_commits > 0:
            return Result(
                status="ALL_MERGED_WITH_NEW_COMMITS",
                action="CREATE_PR",
                summary=f">> CREATE PR — {upstream_commits} new commit(s) after last merge (#{latest_merged['number']}).",
                details=[f"   head: {head} -> base: {base}"],
                dirty=dirty,
                head=head,
                base=base,
                pr_number=latest_merged["number"],
                new_commits=upstream_commits,
                fallback_used=True,
            )
        return Result(
            status="ALL_MERGED_NO_NEW_COMMITS",
            action="NO_ACTION",
            summary=f"-- NO ACTION — All PRs merged, no new commits on `{head}`.",
            details=[],
            dirty=dirty,
            head=head,
            base=base,
            pr_number=latest_merged["number"],
            new_commits=0,
            fallback_used=True,
        )

    base_commits = git_count(repo, f"origin/{base}..HEAD")
    if base_commits is not None:
        if base_commits > 0:
            return Result(
                status="ALL_MERGED_WITH_NEW_COMMITS",
                action="CREATE_PR",
                summary=f">> CREATE PR — {base_commits} new commit(s) after last merge (#{latest_merged['number']}).",
                details=[f"   head: {head} -> base: {base}"],
                dirty=dirty,
                head=head,
                base=base,
                pr_number=latest_merged["number"],
                new_commits=base_commits,
                fallback_used=True,
            )
        return Result(
            status="ALL_MERGED_NO_NEW_COMMITS",
            action="NO_ACTION",
            summary=f"-- NO ACTION — All PRs merged, no new commits on `{head}`.",
            details=[],
            dirty=dirty,
            head=head,
            base=base,
            pr_number=latest_merged["number"],
            new_commits=0,
            fallback_used=True,
        )

    return Result(
        status="CHECK_FAILED",
        action="MANUAL_CHECK",
        summary="!! MANUAL CHECK — Could not determine PR status.",
        details=[
            "   Reason: Could not resolve merge commit and fallback comparison failed",
            f"   head: {head} -> base: {base}",
        ],
        dirty=dirty,
        head=head,
        base=base,
        reason="Could not resolve merge commit and fallback comparison failed",
    )


def main() -> int:
    args = parse_args()
    repo = Path(args.repo).resolve()

    head = args.head or require_success(
        run(["git", "rev-parse", "--abbrev-ref", "HEAD"], repo),
        "resolve head branch",
    )
    dirty = bool(run(["git", "status", "--porcelain"], repo).stdout.strip())

    try:
        result = build_result(repo, args.base, head, dirty)
    except Exception as exc:  # pragma: no cover - user-facing fallback
        result = Result(
            status="CHECK_FAILED",
            action="MANUAL_CHECK",
            summary="!! MANUAL CHECK — Could not determine PR status.",
            details=[f"   Reason: {exc}", f"   head: {head} -> base: {args.base}"],
            dirty=dirty,
            head=head,
            base=args.base,
            reason=str(exc),
        )

    print(format_result(result, args.lang))
    if args.json:
        print()
        print(
            json.dumps(
                {
                    "status": result.status,
                    "action": result.action,
                    "head": result.head,
                    "base": result.base,
                    "dirty": result.dirty,
                    "reason": result.reason,
                    "prNumber": result.pr_number,
                    "prUrl": result.pr_url,
                    "newCommits": result.new_commits,
                    "fallbackUsed": result.fallback_used,
                },
                ensure_ascii=False,
                indent=2,
            )
        )

    return 0 if result.status != "CHECK_FAILED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
