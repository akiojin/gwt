#!/usr/bin/env python3
"""Compute the next release version from latest-tag-relative commit classification.

This is the canonical version-bump logic for the release flow. It deliberately
does NOT use ``git-cliff --bumped-version`` because that recomputes from the full
history and regresses the version on repositories with non-conventional commits
(see SPEC-1932 and `.claude/commands/release.md`). Instead it classifies only the
commits since the latest ``vX.Y.Z`` tag and bumps relative to that tag.

Used by `.github/workflows/prepare-release.yml`. Pure functions are unit tested in
`scripts/test_compute_release_version.py` with an injected command runner.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from typing import Callable, Sequence

CommandRunner = Callable[[Sequence[str]], str]

# A commit carries a breaking marker when the subject has a `!` before the
# colon (e.g. `feat!:`, `fix(x)!:`) or a body line is a `BREAKING CHANGE:` /
# `BREAKING-CHANGE:` footer. The footer alternative is anchored to the line
# start (re.MULTILINE) and requires a trailing space/colon so that prose like
# "see the BREAKING CHANGE section" is not reported as breaking.
#
# Issue #4373: the marker is *reported* (`breaking_commits`) but never decides
# the version. Agents kept writing the footer and the major version climbed
# to v9 without a human decision, so a major release is reachable only through
# an explicit `--bump major` from the person who triggers the release.
BREAKING_RE = re.compile(r"(^[a-z]+(\(.+\))?!:|^BREAKING[ -]CHANGE[ :])", re.MULTILINE)
FEAT_RE = re.compile(r"^feat(\(.+\))?[!:]")
FIX_RE = re.compile(r"^fix(\(.+\))?[!:]")
VERSION_TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")

VALID_BUMPS = ("auto", "patch", "minor", "major")


class ReleaseVersionError(RuntimeError):
    """Raised when a version cannot be computed safely."""


def _default_runner(cmd: Sequence[str]) -> str:
    return subprocess.run(
        list(cmd),
        check=True,
        capture_output=True,
        text=True,
        # git emits UTF-8 regardless of the host locale; decoding with the
        # locale (cp932 on Windows) crashes on non-ASCII commit subjects.
        encoding="utf-8",
        errors="replace",
    ).stdout


def parse_tag(tag: str | None) -> tuple[int, int, int] | None:
    """Parse ``vX.Y.Z`` into a (major, minor, patch) tuple, or None.

    Only stable ``vX.Y.Z`` tags are recognized. Pre-release / build-metadata
    tags (``v1.2.3-rc1``, ``v1.2.3+build``) return None by design — this
    workflow manages stable develop->main releases only.
    """
    if not tag:
        return None
    match = VERSION_TAG_RE.match(tag.strip())
    if not match:
        return None
    return (int(match.group(1)), int(match.group(2)), int(match.group(3)))


def classify_bump(subjects: Sequence[str], combined_body: str) -> str:
    """Classify the release level from commit subjects and the combined body.

    Returns ``minor`` or ``patch`` — never ``major``. A breaking marker or a
    ``feat`` yields minor, everything else patch (docs/chore-only ranges still
    bump patch so the release is never a no-op version). The major level is
    only reachable through an explicit ``bump=major`` (Issue #4373); breaking
    markers are surfaced separately by ``breaking_commits``.
    """
    if BREAKING_RE.search(combined_body):
        return "minor"
    if any(FEAT_RE.match(subject) for subject in subjects):
        return "minor"
    if any(FIX_RE.match(subject) for subject in subjects):
        return "patch"
    return "patch"


def next_version(prev: tuple[int, int, int], level: str) -> str:
    """Bump ``prev`` by ``level`` and return the ``X.Y.Z`` string (no ``v``)."""
    major, minor, patch = prev
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    if level == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise ReleaseVersionError(f"unknown bump level: {level}")


def resolve_version(
    prev_tag: str | None,
    subjects: Sequence[str],
    combined_body: str,
    bump: str,
) -> str:
    """Resolve the next version string from inputs.

    ``bump=auto`` classifies from commits and never emits a major bump: a
    breaking marker caps at minor. A major release requires the person who
    triggers the release to pass ``--bump major`` explicitly (Issue #4373).
    """
    if bump not in VALID_BUMPS:
        raise ReleaseVersionError(f"invalid bump: {bump} (expected one of {', '.join(VALID_BUMPS)})")
    prev = parse_tag(prev_tag) or (0, 0, 0)
    level = classify_bump(subjects, combined_body) if bump == "auto" else bump
    return next_version(prev, level)


def latest_version_tag(runner: CommandRunner) -> str | None:
    out = runner(["git", "tag", "--list", "v[0-9]*", "--sort=-version:refname"])
    for line in out.splitlines():
        if VERSION_TAG_RE.match(line.strip()):
            return line.strip()
    return None


def count_commits(commit_range: str, runner: CommandRunner) -> int:
    """Total number of commits in the range (including merges)."""
    out = runner(["git", "rev-list", "--count", commit_range])
    try:
        return int(out.strip() or "0")
    except ValueError:
        return 0


def gather_commits(commit_range: str, runner: CommandRunner) -> tuple[list[str], str]:
    """Return (subjects, combined-subject+body), both excluding merge commits.

    Merge commits are excluded from both queries so a GitHub-generated merge
    body ("Merge pull request #N from ...") cannot influence classification;
    the real Conventional Commit subjects/footers live on the non-merge commits.
    """
    subjects_out = runner(["git", "log", commit_range, "--pretty=%s", "--no-merges"])
    body_out = runner(["git", "log", commit_range, "--pretty=%s%n%b", "--no-merges"])
    subjects = [line for line in subjects_out.splitlines() if line.strip()]
    return subjects, body_out


# One record per commit: abbreviated sha, subject, body — fields separated by
# US (0x1f) and records by RS (0x1e) so multi-line bodies parse unambiguously.
BREAKING_LOG_FORMAT = "%h%x1f%s%x1f%b%x1e"


def parse_breaking_commits(log_output: str) -> list[str]:
    """Return ``"<sha> <subject>"`` for every record carrying a breaking marker."""
    found: list[str] = []
    for record in log_output.split("\x1e"):
        parts = record.lstrip("\r\n").split("\x1f", 2)
        if len(parts) < 2 or not parts[0].strip():
            continue
        sha, subject = parts[0].strip(), parts[1].strip()
        body = parts[2] if len(parts) == 3 else ""
        if BREAKING_RE.search(f"{subject}\n{body}"):
            found.append(f"{sha} {subject}")
    return found


def breaking_commits(commit_range: str, runner: CommandRunner) -> list[str]:
    """List non-merge commits in the range that carry a breaking marker.

    The result is informational (Issue #4373 AC-2): the workflow logs it and
    the Release PR body lists it, but it never raises the bump level.
    """
    out = runner(["git", "log", commit_range, f"--pretty={BREAKING_LOG_FORMAT}", "--no-merges"])
    return parse_breaking_commits(out)


def resolve_range(
    commit_range: str | None,
    prev_tag: str | None,
    runner: CommandRunner,
) -> tuple[str | None, str]:
    """Fill in the latest tag and the ``<tag>..HEAD`` range when not given."""
    if prev_tag is None:
        prev_tag = latest_version_tag(runner)
    if commit_range is None:
        commit_range = f"{prev_tag}..HEAD" if prev_tag else "HEAD"
    return prev_tag, commit_range


def list_breaking(
    commit_range: str | None = None,
    prev_tag: str | None = None,
    runner: CommandRunner | None = None,
) -> list[str]:
    """End-to-end: resolve the range from git and list its breaking commits."""
    runner = runner or _default_runner
    _, commit_range = resolve_range(commit_range, prev_tag, runner)
    return breaking_commits(commit_range, runner)


def compute(
    bump: str,
    commit_range: str | None = None,
    prev_tag: str | None = None,
    runner: CommandRunner | None = None,
) -> str:
    """End-to-end: resolve the latest tag + range from git and compute the version."""
    runner = runner or _default_runner
    prev_tag, commit_range = resolve_range(commit_range, prev_tag, runner)
    # Refuse an empty release: a range with zero commits would still bump the
    # version (classify_bump defaults to patch), producing a tag with no content.
    if count_commits(commit_range, runner) == 0:
        raise ReleaseVersionError(
            f"no new commits in range {commit_range}; nothing to release"
        )
    subjects, body = gather_commits(commit_range, runner)
    return resolve_version(prev_tag, subjects, body, bump)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Compute next release version (latest-tag relative).")
    parser.add_argument("--bump", choices=VALID_BUMPS, default="auto")
    parser.add_argument("--range", dest="commit_range", default=None, help="git revision range, e.g. v9.54.0..HEAD")
    parser.add_argument("--prev-tag", dest="prev_tag", default=None, help="override the latest tag")
    parser.add_argument(
        "--list-breaking",
        action="store_true",
        help="print the breaking-marker commits in the range (one `<sha> <subject>` per line) instead of a version",
    )
    args = parser.parse_args(argv)
    if args.list_breaking:
        for line in list_breaking(commit_range=args.commit_range, prev_tag=args.prev_tag):
            print(line)
        return 0
    try:
        version = compute(args.bump, commit_range=args.commit_range, prev_tag=args.prev_tag)
    except ReleaseVersionError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    print(version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
