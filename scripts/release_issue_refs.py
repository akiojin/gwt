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
from urllib.parse import quote, unquote, urlsplit

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



FEEDBACK_REPO = "akiojin/gwt"
FEEDBACK_AUTHORS = {"akiojin", "github-actions[bot]"}
FEEDBACK_TAG_RE = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")
FEEDBACK_MARKER_RE = re.compile(
    r"<!-- gwt-feedback:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12} -->", re.I
)
SOURCE_MARKER_RE = re.compile(r"^<!-- gwt-feedback-source:([1-9][0-9]*) -->$", re.M)
OWNER_MARKER_RE = re.compile(r"^<!-- gwt-feedback-owner:([1-9][0-9]*):([1-9][0-9]*) -->(?:\r?\n|$)")


def _feedback_api(path: str, runner: CommandRunner, body: str | None = None):
    command = ["gh", "api", "--hostname", "github.com", "--method",
               "GET" if body is None else "POST", f"repos/{FEEDBACK_REPO}/{path}"]
    if body is not None:
        command.extend(["-f", f"body={body}"])
    return json.loads(runner(command))


def _feedback_author(payload: dict) -> str:
    author = (payload.get("user") or {}).get("login")
    if not isinstance(author, str) or not author:
        raise ValueError("feedback provenance has unknown author")
    return author.lower()


def _feedback_comments(number: int, runner: CommandRunner) -> list[dict]:
    comments = []
    seen = set()
    page = 1
    while True:
        try:
            batch = _feedback_api(f"issues/{number}/comments?per_page=100&page={page}", runner)
        except (subprocess.CalledProcessError, ValueError) as error:
            raise RuntimeError(f"notification pending: #{number} comments unverified (page {page})") from error
        if not isinstance(batch, list) or len(batch) > 100:
            raise ValueError(f"#{number} comment pagination is unverified")
        for comment in batch:
            if not isinstance(comment, dict):
                raise ValueError(f"#{number} comment provenance is unverified")
            _feedback_author(comment)
            identity = comment.get("id")
            if not isinstance(identity, int) or identity <= 0 or identity in seen:
                raise ValueError(f"#{number} comment pagination is unverified")
            seen.add(identity)
        comments.extend(batch)
        if len(batch) < 100:
            return comments
        page += 1


def _raw_feedback(issue: dict) -> bool:
    return "<!-- gwt-feedback:" in (issue.get("body") or "").lower() or any(
        label.get("name", "").lower() == "gwt-feedback" for label in issue.get("labels", [])
    )


def _trusted_notice(comment: dict, marker: str) -> bool:
    return (_feedback_author(comment) in FEEDBACK_AUTHORS
            and (comment.get("body") or "").splitlines()[:1] == [marker])


def feedback_release_range(tag: str, runner: CommandRunner = run_command) -> str:
    """Use the nearest published ancestor; failed draft releases are not a boundary."""
    if not FEEDBACK_TAG_RE.fullmatch(tag):
        raise ValueError("invalid published release tag")
    published = set()
    seen = set()
    page = 1
    while True:
        batch = _feedback_api(f"releases?per_page=100&page={page}", runner)
        if not isinstance(batch, list) or len(batch) > 100:
            raise ValueError("release pagination is unverified")
        for release in batch:
            identity = release.get("id")
            if not isinstance(identity, int) or identity <= 0 or identity in seen:
                raise ValueError("release pagination is unverified")
            seen.add(identity)
            name = release.get("tag_name") or ""
            if (release.get("draft") is False and release.get("published_at")
                    and name != tag and FEEDBACK_TAG_RE.fullmatch(name)):
                published.add(name)
        if len(batch) < 100:
            break
        page += 1
    ancestors = set(runner(["git", "tag", "--merged", tag]).splitlines())
    eligible = sorted(published & ancestors)
    if not eligible:
        return tag
    command = ["git", "describe", "--tags", "--abbrev=0"]
    for name in eligible:
        command.extend(["--match", name])
    previous = runner([*command, tag]).strip()
    if previous not in eligible:
        raise ValueError("previous published release is unverified")
    return f"{previous}..{tag}"


def notify_feedback_results(
    report: ReleaseIssueRefs, tag: str, runner: CommandRunner = run_command,
) -> list[str]:
    """Notify through one trusted owner edge, after publication, without closing Issues.

    All reads/writes use the fixed public repository. Reads must complete before
    absence is trusted; a lost POST response is reconciled rather than retried.
    Release workflow serialization supplies the single-writer boundary.
    """
    if report.repo != FEEDBACK_REPO:
        raise ValueError("feedback notifications are restricted to akiojin/gwt")
    if not FEEDBACK_TAG_RE.fullmatch(tag):
        raise ValueError("invalid published release tag")
    if report.range != tag and not report.range.endswith(f"..{tag}"):
        raise ValueError("notification range must end at the published release tag")
    release = _feedback_api(f"releases/tags/{quote(tag, safe='')}", runner)
    url = urlsplit(release.get("html_url") or "")
    if (release.get("draft") is not False or not release.get("published_at")
            or release.get("tag_name") != tag or url.scheme != "https"
            or url.netloc != "github.com" or url.query or url.fragment
            or unquote(url.path) != f"/{FEEDBACK_REPO}/releases/tag/{tag}"):
        raise ValueError("release must be published, non-draft, and in akiojin/gwt")

    commits = set(runner(["git", "rev-list", "--end-of-options", report.range]).splitlines())
    owner_prs: dict[int, list[tuple[int, str, bool]]] = {}
    # Both per-PR lists nominate owners; Related-only refs require independent
    # settlement proof and never inherit authority from the PR author alone.
    for ref in report.refs:
        candidates = unique_sorted([*ref.delivered_issues, *ref.reference_only_issues])
        if ref.kind != "pr" or not candidates:
            continue
        pr = _feedback_api(f"pulls/{ref.number}", runner)
        if not pr.get("merged_at") or pr.get("merge_commit_sha") not in commits:
            raise ValueError(f"PR #{ref.number} is not merged in the published release range")
        author = (pr.get("user") or {}).get("login")
        trusted_author = isinstance(author, str) and author.lower() in FEEDBACK_AUTHORS
        for owner in candidates:
            owner_prs.setdefault(owner, []).append((
                ref.number, pr["merge_commit_sha"], trusted_author and owner in ref.delivered_issues))

    messages = []
    for owner, deliveries in sorted(owner_prs.items()):
        issue = _feedback_api(f"issues/{owner}", runner)
        if issue.get("number") != owner:
            raise ValueError(f"#{owner} owner identity is unverified")
        if issue.get("pull_request"):
            messages.append(f"Skipped #{owner}: pull request is not an Issue owner")
            continue
        if _feedback_author(issue) not in FEEDBACK_AUTHORS or _raw_feedback(issue):
            messages.append(f"Skipped #{owner}: not a trusted curated owner")
            continue
        comments = _feedback_comments(owner, runner)
        prs = []
        for number, merge_sha, trusted_author in deliveries:
            marker = f"<!-- gwt-merged-issue-settlement v1 pr={number} sha={merge_sha} -->"
            if trusted_author or any(
                _feedback_author(comment) in FEEDBACK_AUTHORS
                and (comment.get("body") or "").splitlines()[:1] == [marker]
                for comment in comments
            ):
                prs.append(number)
        if not prs:
            messages.append(f"Skipped #{owner}: PR delivery linkage is unverified")
            continue
        targets = {int(match.group(1)) for match in SOURCE_MARKER_RE.finditer(issue.get("body") or "")}
        for comment in comments:
            if _feedback_author(comment) in FEEDBACK_AUTHORS:
                targets.update(int(match.group(1)) for match in OWNER_MARKER_RE.finditer(comment.get("body") or "")
                               if int(match.group(2)) == owner)
        targets.discard(owner)
        # A report already aggregated on a trusted owner needs no reverse edge.
        if any(FEEDBACK_MARKER_RE.match((comment.get("body") or "").lstrip()) for comment in comments):
            targets.add(owner)
        for target in sorted(targets):
            target_comments = comments if target == owner else _feedback_comments(target, runner)
            if target != owner:
                source = _feedback_api(f"issues/{target}", runner)
                if source.get("number") != target or source.get("pull_request") or not _raw_feedback(source):
                    raise ValueError(f"#{target} source report identity is unverified")
            marker = f"<!-- gwt-feedback-release:{target}:{owner}:{tag} -->"
            if any(_trusted_notice(comment, marker) for comment in target_comments):
                messages.append(f"Already recorded {tag} on #{target} for #{owner}")
                continue
            phase = any(label.get("name") == SPEC_LABEL for label in issue.get("labels", []))
            scope = "SPEC phase" if phase else "curated change"
            links = ", ".join(f"https://github.com/{FEEDBACK_REPO}/pull/{pr}" for pr in sorted(set(prs)))
            body = (f"{marker}\nPublished {tag}: {release['html_url']}\n\n"
                    f"The {scope} for https://github.com/{FEEDBACK_REPO}/issues/{owner} "
                    f"is included through merged PR(s): {links}.\n"
                    "This records the specified change; it does not mark the entire SPEC or report resolved.")
            try:
                posted = _feedback_api(f"issues/{target}/comments", runner, body)
                if not isinstance(posted, dict) or not _trusted_notice(posted, marker):
                    raise ValueError("notification POST result is unverified")
            except (subprocess.CalledProcessError, ValueError) as error:
                readback = _feedback_comments(target, runner)
                if not any(_trusted_notice(comment, marker) for comment in readback):
                    raise RuntimeError(f"published release {tag}; notification pending/unverified on #{target}") from error
            messages.append(f"Recorded {tag} on #{target} for #{owner}")
    return messages


def _fixed_github_collection_command(args: Sequence[str]) -> str:
    """Keep collector reads on the same host as notification writes."""
    command = list(args)
    if command[:2] == ["gh", "api"]:
        command[2:2] = ["--hostname", "github.com"]
    elif command[:3] == ["gh", "pr", "view"]:
        command[command.index("--repo") + 1] = f"https://github.com/{FEEDBACK_REPO}"
    return run_command(command)


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
    breaking_commits: Sequence[str] | None = None,
) -> str:
    """Render the reference-only Release PR body (Issue #3545 AC-1 / AC-2).

    Delivered Issues are listed as bare `#N` references. Every line, including
    free-text `notes`, passes through `neutralize_closing_keywords`, and the
    result is asserted to contain no closing reference before it is returned.

    `breaking_commits` (Issue #4373 AC-2) lists commits that carry a breaking
    marker. They are informational only: the marker no longer raises the
    version, so the section says so and names the explicit `bump=major` route.
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
    if breaking_commits:
        sections.extend(
            [
                "## Breaking Changes",
                "",
                "Breaking markers (`type!:` / `BREAKING CHANGE:`) were found in this range but are "
                "not applied to the version (Issue #4373): a major release requires an explicit "
                "`bump=major` from the person triggering Prepare Release.",
                "",
                *(f"- {line.strip()}" for line in breaking_commits if line.strip()),
                "",
            ]
        )
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
    parser.add_argument(
        "--breaking-file",
        dest="breaking_file",
        default=None,
        help="Optional file with one `<sha> <subject>` breaking commit per line, listed under `## Breaking Changes`.",
    )
    parser.add_argument("--notify-feedback", action="store_true",
                        help="Record published release results on linked feedback (akiojin/gwt only); --range TAG selects the prior published release automatically.")
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if args.format == "pr-body" and not args.version:
        parser.error("--format pr-body requires --version")

    if args.notify_feedback:
        if not args.version or args.repo_slug not in (None, FEEDBACK_REPO):
            parser.error("--notify-feedback requires --version and fixed --repo akiojin/gwt")
        try:
            range_expr = (feedback_release_range(args.version)
                          if args.range_expr == args.version else args.range_expr)
            report = collect_release_issue_refs(range_expr, repo_slug=FEEDBACK_REPO,
                                                runner=_fixed_github_collection_command)
            for message in notify_feedback_results(report, args.version):
                print(message)
            return 0
        except (subprocess.CalledProcessError, ValueError, RuntimeError) as error:
            print(f"Release report notification pending/unverified: {error}", file=sys.stderr)
            return 1

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
        breaking: list[str] = []
        if args.breaking_file:
            with open(args.breaking_file, encoding="utf-8") as handle:
                breaking = [line for line in handle.read().splitlines() if line.strip()]
        print(
            render_release_pr_body(
                report, args.version, args.bump, notes=notes, breaking_commits=breaking
            ),
            end="",
        )
    else:
        print(render_text(report))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
