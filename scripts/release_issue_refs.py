#!/usr/bin/env python3
"""Collect auto-close and reference-only issue refs for a release range."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from typing import Callable, Sequence

CommandRunner = Callable[[Sequence[str]], str]

CLOSING_KEYWORD_RE = re.compile(
    r"\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)\b",
    re.IGNORECASE,
)
ISSUE_REF_RE = re.compile(r"#(\d+)")
SQUASH_REF_RE = re.compile(r"\(#(\d+)\)$")
MERGE_PR_RE = re.compile(r"^Merge pull request #(\d+)\b")
HEADING_RE = re.compile(r"^##\s+(?P<title>.+?)\s*$")


@dataclass
class BodyIssueRefs:
    auto_close_issues: list[int] = field(default_factory=list)
    reference_only_issues: list[int] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


@dataclass
class CommitRef:
    number: int
    kind: str
    source: str
    auto_close_issues: list[int] = field(default_factory=list)
    reference_only_issues: list[int] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


@dataclass
class ReleaseIssueRefs:
    repo: str
    range: str
    refs: list[CommitRef] = field(default_factory=list)
    auto_close_issues: list[int] = field(default_factory=list)
    reference_only_issues: list[int] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


def run_command(args: Sequence[str]) -> str:
    result = subprocess.run(args, check=True, capture_output=True, text=True)
    return result.stdout


def unique_sorted(numbers: Sequence[int | str]) -> list[int]:
    return sorted({int(value) for value in numbers})


def dedupe_preserve_order(items: Sequence[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for item in items:
        if item not in seen:
            seen.add(item)
            ordered.append(item)
    return ordered


def format_issue_refs(numbers: Sequence[int]) -> str:
    return ", ".join(f"#{number}" for number in unique_sorted(numbers))


def extract_section(body: str, section_title: str) -> str:
    lines = body.splitlines()
    in_section = False
    collected: list[str] = []

    for line in lines:
        heading = HEADING_RE.match(line.strip())
        if heading:
            title = heading.group("title").strip()
            if in_section:
                break
            in_section = title == section_title
            continue

        if in_section:
            collected.append(line)

    return "\n".join(collected)


def extract_issue_numbers(text: str) -> list[int]:
    return unique_sorted(match.group(1) for match in ISSUE_REF_RE.finditer(text or ""))


def parse_pr_body_refs(body: str, pr_number: int | None = None) -> BodyIssueRefs:
    closing_section = extract_section(body, "Closing Issues")
    related_section = extract_section(body, "Related Issues / Links")

    auto_close = set(extract_issue_numbers(closing_section))
    auto_close.update(int(match.group(1)) for match in CLOSING_KEYWORD_RE.finditer(body or ""))

    reference_only = set(extract_issue_numbers(related_section)) - auto_close
    warnings: list[str] = []

    if reference_only:
        prefix = f"PR #{pr_number}" if pr_number is not None else "PR body"
        warnings.append(
            f"{prefix} references {format_issue_refs(sorted(reference_only))} only in "
            "`Related Issues / Links`; they will not auto-close on release."
        )

    return BodyIssueRefs(
        auto_close_issues=sorted(auto_close),
        reference_only_issues=sorted(reference_only),
        warnings=warnings,
    )


def extract_release_commit_refs(
    no_merge_subjects: str,
    merge_subjects: str,
) -> list[tuple[int, str]]:
    refs: dict[int, str] = {}

    for subject in no_merge_subjects.splitlines():
        match = SQUASH_REF_RE.search(subject.strip())
        if match:
            refs.setdefault(int(match.group(1)), "squash")

    for subject in merge_subjects.splitlines():
        match = MERGE_PR_RE.search(subject.strip())
        if match:
            refs.setdefault(int(match.group(1)), "merge")

    return [(number, refs[number]) for number in sorted(refs)]


def resolve_repo_slug(runner: CommandRunner) -> str:
    return runner(
        ["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"]
    ).strip()


def fetch_issue_labels(number: int, repo_slug: str, runner: CommandRunner) -> list[str]:
    """Return label names for a GitHub issue."""
    payload = json.loads(runner(["gh", "api", f"repos/{repo_slug}/issues/{number}"]) or "{}")
    return [label["name"] for label in payload.get("labels", [])]


SPEC_LABEL = "gwt-spec"


def classify_release_ref(
    number: int,
    source: str,
    repo_slug: str,
    runner: CommandRunner,
) -> CommitRef:
    issue_payload = json.loads(runner(["gh", "api", f"repos/{repo_slug}/issues/{number}"]) or "{}")
    if issue_payload.get("pull_request"):
        pr_payload = json.loads(
            runner(["gh", "pr", "view", str(number), "--repo", repo_slug, "--json", "body"]) or "{}"
        )
        pr_refs = parse_pr_body_refs(pr_payload.get("body") or "", pr_number=number)
        return CommitRef(
            number=number,
            kind="pr",
            source=source,
            auto_close_issues=pr_refs.auto_close_issues,
            reference_only_issues=pr_refs.reference_only_issues,
            warnings=pr_refs.warnings,
        )

    return CommitRef(
        number=number,
        kind="issue",
        source=source,
        auto_close_issues=[number],
    )


def collect_release_issue_refs(
    range_expr: str,
    repo_slug: str | None = None,
    runner: CommandRunner = run_command,
) -> ReleaseIssueRefs:
    repo = repo_slug or resolve_repo_slug(runner)
    no_merge_subjects = runner(["git", "log", "--pretty=%s", "--no-merges", range_expr])
    merge_subjects = runner(["git", "log", "--merges", "--pretty=%s", range_expr])

    refs: list[CommitRef] = []
    auto_close: set[int] = set()
    reference_only: set[int] = set()
    warnings: list[str] = []

    for number, source in extract_release_commit_refs(no_merge_subjects, merge_subjects):
        ref = classify_release_ref(number, source, repo, runner)
        refs.append(ref)
        auto_close.update(ref.auto_close_issues)
        reference_only.update(ref.reference_only_issues)
        warnings.extend(ref.warnings)

    # Post-filter: move gwt-spec issues from auto-close to reference-only
    spec_protected: list[int] = []
    for issue_number in sorted(auto_close):
        labels = fetch_issue_labels(issue_number, repo, runner)
        if SPEC_LABEL in labels:
            spec_protected.append(issue_number)

    if spec_protected:
        auto_close.difference_update(spec_protected)
        reference_only.update(spec_protected)
        warnings.append(
            f"gwt-spec issues moved to reference-only: "
            f"{format_issue_refs(spec_protected)}. "
            "gwt-spec issues are never auto-closed by releases."
        )

    reference_only.difference_update(auto_close)
    if reference_only:
        warnings.insert(
            0,
            "Reference-only issues detected: "
            f"{format_issue_refs(sorted(reference_only))}. "
            "Add them to `Closing Issues` if they should auto-close.",
        )

    return ReleaseIssueRefs(
        repo=repo,
        range=range_expr,
        refs=refs,
        auto_close_issues=sorted(auto_close),
        reference_only_issues=sorted(reference_only),
        warnings=dedupe_preserve_order(warnings),
    )


def render_text(report: ReleaseIssueRefs) -> str:
    lines = [
        f"Repo: {report.repo}",
        f"Range: {report.range}",
        "",
        "Auto-close issues:",
    ]

    if report.auto_close_issues:
        lines.extend(f"- Closes #{number}" for number in report.auto_close_issues)
    else:
        lines.append("- None")

    lines.extend(["", "Reference-only issues:"])
    if report.reference_only_issues:
        lines.extend(f"- #{number}" for number in report.reference_only_issues)
    else:
        lines.append("- None")

    if report.warnings:
        lines.extend(["", "Warnings:"])
        lines.extend(f"- {warning}" for warning in report.warnings)

    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Collect release auto-close and reference-only issue refs."
    )
    parser.add_argument(
        "--range",
        dest="range_expr",
        required=True,
        help="Git revision range to inspect, for example v1.2.3..HEAD or HEAD.",
    )
    parser.add_argument(
        "--repo",
        dest="repo_slug",
        default=None,
        help="GitHub repo slug (owner/name). Defaults to `gh repo view`.",
    )
    parser.add_argument(
        "--format",
        choices=("text", "json"),
        default="text",
        help="Output format.",
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    try:
        report = collect_release_issue_refs(args.range_expr, repo_slug=args.repo_slug)
    except subprocess.CalledProcessError as error:
        stderr = (error.stderr or "").strip()
        if stderr:
            print(stderr, file=sys.stderr)
        else:
            print(str(error), file=sys.stderr)
        return error.returncode or 1

    if args.format == "json":
        print(json.dumps(asdict(report), ensure_ascii=False, indent=2))
    else:
        print(render_text(report))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
