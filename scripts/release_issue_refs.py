#!/usr/bin/env python3
"""Collect delivered and reference-only issue refs for a release range.

The Release PR (develop -> main) must never carry GitHub closing keywords:
`main` is the default branch, so `Closes #N` in its body closes the Issue on
merge even when acceptance criteria are still open (Issue #3545). Issues are
settled by the Issue Monitor when the work branch merges into `develop`
(Issue #3917); the Release PR only *references* them.
"""

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
# GitHub closing-keyword grammar: keyword, optional colon, whitespace, then a
# `#N`, `owner/repo#N`, or issue-URL reference. Anything matching this in a
# body pushed to the default branch closes the Issue.
CLOSING_REFERENCE_RE = re.compile(
    r"(?P<keyword>\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?))"
    r"(?P<sep>:?\s+)"
    r"(?P<ref>(?:[\w.-]+/[\w.-]+)?#\d+\b|https://github\.com/[\w.-]+/[\w.-]+/issues/\d+\b)",
    re.IGNORECASE,
)
ISSUE_REF_RE = re.compile(r"#(\d+)")
SQUASH_REF_RE = re.compile(r"\(#(\d+)\)$")
MERGE_PR_RE = re.compile(r"^Merge pull request #(\d+)\b")
HEADING_RE = re.compile(r"^##\s+(?P<title>.+?)\s*$")


@dataclass
class BodyIssueRefs:
    delivered_issues: list[int] = field(default_factory=list)
    reference_only_issues: list[int] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


@dataclass
class CommitRef:
    number: int
    kind: str
    source: str
    delivered_issues: list[int] = field(default_factory=list)
    reference_only_issues: list[int] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


@dataclass
class ReleaseIssueRefs:
    repo: str
    range: str
    refs: list[CommitRef] = field(default_factory=list)
    delivered_issues: list[int] = field(default_factory=list)
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

    delivered = set(extract_issue_numbers(closing_section))
    delivered.update(int(match.group(1)) for match in CLOSING_KEYWORD_RE.finditer(body or ""))

    reference_only = set(extract_issue_numbers(related_section)) - delivered
    warnings: list[str] = []

    if reference_only:
        prefix = f"PR #{pr_number}" if pr_number is not None else "PR body"
        warnings.append(
            f"{prefix} references {format_issue_refs(sorted(reference_only))} only in "
            "`Related Issues / Links`; listed as reference-only."
        )

    return BodyIssueRefs(
        delivered_issues=sorted(delivered),
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
            delivered_issues=pr_refs.delivered_issues,
            reference_only_issues=pr_refs.reference_only_issues,
            warnings=pr_refs.warnings,
        )

    return CommitRef(
        number=number,
        kind="issue",
        source=source,
        delivered_issues=[number],
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
    delivered: set[int] = set()
    reference_only: set[int] = set()
    warnings: list[str] = []

    for number, source in extract_release_commit_refs(no_merge_subjects, merge_subjects):
        ref = classify_release_ref(number, source, repo, runner)
        refs.append(ref)
        delivered.update(ref.delivered_issues)
        reference_only.update(ref.reference_only_issues)
        warnings.extend(ref.warnings)

    # Post-filter: gwt-spec issues are settled per phase, so they are only
    # referenced by a release, never listed as delivered.
    spec_protected: list[int] = []
    for issue_number in sorted(delivered):
        labels = fetch_issue_labels(issue_number, repo, runner)
        if SPEC_LABEL in labels:
            spec_protected.append(issue_number)

    if spec_protected:
        delivered.difference_update(spec_protected)
        reference_only.update(spec_protected)
        warnings.append(
            f"gwt-spec issues moved to reference-only: "
            f"{format_issue_refs(spec_protected)}. "
            "gwt-spec issues are settled per phase, never by a release."
        )

    reference_only.difference_update(delivered)
    if reference_only:
        warnings.insert(
            0,
            "Reference-only issues detected: "
            f"{format_issue_refs(sorted(reference_only))}. "
            "They are listed under `Related Issues / Links`.",
        )

    return ReleaseIssueRefs(
        repo=repo,
        range=range_expr,
        refs=refs,
        delivered_issues=sorted(delivered),
        reference_only_issues=sorted(reference_only),
        warnings=dedupe_preserve_order(warnings),
    )


def neutralize_closing_keywords(text: str) -> str:
    """Break every `keyword #N` pair so GitHub cannot read it as a closing link.

    The reference is wrapped in a code span: GitHub does not autolink inside
    code, so the keyword no longer precedes a linked reference. The rewrite
    is idempotent and leaves plain `#N` references untouched.
    """

    def wrap(match: re.Match[str]) -> str:
        return f"{match.group('keyword')}{match.group('sep')}`{match.group('ref')}`"

    return CLOSING_REFERENCE_RE.sub(wrap, text or "")


def contains_closing_reference(text: str) -> bool:
    """True when `text` still carries a GitHub closing keyword + reference."""
    return CLOSING_REFERENCE_RE.search(text or "") is not None


def _issue_list_section(numbers: Sequence[int]) -> str:
    if not numbers:
        return "None"
    return "\n".join(f"- #{number}" for number in unique_sorted(numbers))


def render_release_pr_body(
    report: ReleaseIssueRefs,
    version: str,
    bump: str,
    notes: str | None = None,
) -> str:
    """Render the reference-only Release PR body (Issue #3545 AC-1 / AC-2).

    Delivered Issues are listed as bare `#N` references. Every line, including
    free-text `notes`, passes through `neutralize_closing_keywords`, and the
    result is asserted to contain no closing reference before it is returned.
    """
    sections = [
        "## Summary",
        "",
        f"Release {version}. Version bump and CHANGELOG generated by the Prepare Release workflow.",
        "",
        "## Version",
        "",
        f"- {version} (bump: {bump})",
        "",
    ]
    if notes and notes.strip():
        sections.extend(["## Changes", "", notes.strip(), ""])
    sections.extend(
        [
            "## Delivered Issues",
            "",
            _issue_list_section(report.delivered_issues),
            "",
            "Reference-only on purpose: Issues are settled by the Issue Monitor when their "
            "work merges into `develop` (Issue #3917). A Release PR must not close Issues "
            "(Issue #3545).",
            "",
            "## Related Issues / Links",
            "",
            _issue_list_section(report.reference_only_issues),
            "",
        ]
    )
    body = neutralize_closing_keywords("\n".join(sections))
    if contains_closing_reference(body):
        raise ValueError("release PR body still contains a closing keyword reference")
    return body


def render_text(report: ReleaseIssueRefs) -> str:
    lines = [
        f"Repo: {report.repo}",
        f"Range: {report.range}",
        "",
        "Delivered issues:",
    ]

    if report.delivered_issues:
        lines.extend(f"- #{number}" for number in report.delivered_issues)
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
        description="Collect delivered and reference-only issue refs for a release range."
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
        choices=("text", "json", "pr-body"),
        default="text",
        help="Output format. `pr-body` renders the reference-only Release PR body.",
    )
    parser.add_argument(
        "--version",
        dest="version",
        default=None,
        help="Release tag (for example v9.92.0). Required with --format pr-body.",
    )
    parser.add_argument(
        "--bump",
        dest="bump",
        default="auto",
        help="Bump level recorded in the Release PR body (default: auto).",
    )
    parser.add_argument(
        "--notes-file",
        dest="notes_file",
        default=None,
        help="Optional Markdown file appended as `## Changes`; closing keywords are neutralized.",
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if args.format == "pr-body" and not args.version:
        parser.error("--format pr-body requires --version")

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
    elif args.format == "pr-body":
        notes = None
        if args.notes_file:
            with open(args.notes_file, encoding="utf-8") as handle:
                notes = handle.read()
        print(render_release_pr_body(report, args.version, args.bump, notes=notes), end="")
    else:
        print(render_text(report))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
