from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("release_close_guard.py")
MODULE_SPEC = importlib.util.spec_from_file_location("release_close_guard", MODULE_PATH)
assert MODULE_SPEC is not None
assert MODULE_SPEC.loader is not None
release_close_guard = importlib.util.module_from_spec(MODULE_SPEC)
sys.modules[MODULE_SPEC.name] = release_close_guard
MODULE_SPEC.loader.exec_module(release_close_guard)

REPO = "akiojin/gwt"
MERGE_SHA = "fe2133f6e26d4c66db260131888e09ca67ea209c"
MERGED_AT = "2026-09-06T12:15:04Z"


class FakeRunner:
    def __init__(self, outputs: dict[tuple[str, ...], str]) -> None:
        self.outputs = outputs
        self.calls: list[tuple[str, ...]] = []

    def __call__(self, args) -> str:
        key = tuple(args)
        self.calls.append(key)
        if key not in self.outputs:
            raise AssertionError(f"Unexpected command: {key!r}")
        return self.outputs[key]


def pulls_for_commit() -> tuple[str, ...]:
    return ("gh", "api", f"repos/{REPO}/commits/{MERGE_SHA}/pulls")


def closed_issue(number: int, closed_at: str, closer: dict | None) -> dict:
    return {
        "number": number,
        "title": f"issue {number}",
        "closedAt": closed_at,
        "timelineItems": {"nodes": [{"createdAt": closed_at, "closer": closer}]},
    }


def graphql_key() -> tuple[str, ...]:
    return (
        "gh",
        "api",
        "graphql",
        "-f",
        f"query={release_close_guard.CLOSED_ISSUES_QUERY}",
        "-F",
        "owner=akiojin",
        "-F",
        "name=gwt",
    )


class DetectionTests(unittest.TestCase):
    def test_only_issues_closed_by_the_release_merge_are_detected(self) -> None:
        issues = [
            # Closed by the Release PR itself at merge time -> unintended.
            closed_issue(3972, "2026-09-06T12:15:05Z", {"__typename": "PullRequest", "number": 4026}),
            # Closed by a commit message landing on main with the merge -> unintended.
            closed_issue(3973, "2026-09-06T12:15:06Z", {"__typename": "Commit", "oid": "abc123"}),
            # Closed earlier through the API (Issue Monitor settlement) -> intended.
            closed_issue(3917, "2026-09-05T17:48:55Z", None),
            # Closed after the merge by a human -> intended.
            closed_issue(3990, "2026-09-06T13:00:00Z", None),
            # Closed by another PR after the merge -> intended.
            closed_issue(3991, "2026-09-06T13:05:00Z", {"__typename": "PullRequest", "number": 4030}),
        ]

        detected = release_close_guard.detect_release_closures(
            issues, release_pr=4026, merged_at=MERGED_AT
        )

        self.assertEqual([3972, 3973], [issue.number for issue in detected])
        self.assertEqual("PR #4026", detected[0].closer)
        self.assertEqual("commit abc123", detected[1].closer)


class GuardRunTests(unittest.TestCase):
    def outputs(self, issues) -> dict[tuple[str, ...], str]:
        return {
            pulls_for_commit(): json.dumps(
                [
                    {
                        "number": 4026,
                        "merged_at": MERGED_AT,
                        "merge_commit_sha": MERGE_SHA,
                        "base": {"ref": "main"},
                    }
                ]
            ),
            graphql_key(): json.dumps(
                {"data": {"repository": {"issues": {"nodes": issues}}}}
            ),
        }

    def test_reopens_detected_issues_with_a_marker_comment(self) -> None:
        issues = [
            closed_issue(3972, "2026-09-06T12:15:05Z", {"__typename": "PullRequest", "number": 4026}),
            closed_issue(3917, "2026-09-05T17:48:55Z", None),
        ]
        runner = FakeRunner(self.outputs(issues))
        reopen_key = (
            "gh",
            "issue",
            "reopen",
            "3972",
            "--repo",
            REPO,
            "--comment",
            release_close_guard.render_reopen_comment(3972, 4026, MERGE_SHA, "PR #4026"),
        )
        runner.outputs[reopen_key] = "Reopened issue #3972\n"

        report = release_close_guard.run_guard(
            merge_sha=MERGE_SHA, repo_slug=REPO, dry_run=False, runner=runner
        )

        self.assertEqual(4026, report.release_pr)
        self.assertEqual([3972], [issue.number for issue in report.detected])
        self.assertEqual([3972], report.reopened)
        self.assertEqual([], report.failed)
        self.assertIn(reopen_key, runner.calls)
        comment = reopen_key[-1]
        self.assertIn("<!-- gwt-release-close-guard v1 pr=4026 sha=fe2133f6e26d4c66db260131888e09ca67ea209c -->", comment)
        self.assertIn("Issue #3545", comment)

    def test_dry_run_reports_without_mutating(self) -> None:
        issues = [
            closed_issue(3972, "2026-09-06T12:15:05Z", {"__typename": "PullRequest", "number": 4026}),
        ]
        runner = FakeRunner(self.outputs(issues))

        report = release_close_guard.run_guard(
            merge_sha=MERGE_SHA, repo_slug=REPO, dry_run=True, runner=runner
        )

        self.assertEqual([3972], [issue.number for issue in report.detected])
        self.assertEqual([], report.reopened)
        self.assertFalse(any(call[:3] == ("gh", "issue", "reopen") for call in runner.calls))
        text = release_close_guard.render_text(report)
        self.assertIn("#3972", text)
        self.assertIn("dry-run", text)

    def test_no_release_pr_for_commit_is_a_noop(self) -> None:
        runner = FakeRunner({pulls_for_commit(): "[]"})

        report = release_close_guard.run_guard(
            merge_sha=MERGE_SHA, repo_slug=REPO, dry_run=False, runner=runner
        )

        self.assertIsNone(report.release_pr)
        self.assertEqual([], report.detected)
        self.assertEqual(0, release_close_guard.exit_code(report))

    def test_reopen_failure_is_reported_and_fails_the_guard(self) -> None:
        issues = [
            closed_issue(3972, "2026-09-06T12:15:05Z", {"__typename": "PullRequest", "number": 4026}),
        ]

        class FailingRunner(FakeRunner):
            def __call__(self, args) -> str:
                if tuple(args)[:3] == ("gh", "issue", "reopen"):
                    self.calls.append(tuple(args))
                    raise release_close_guard.CommandError("gh: HTTP 403")
                return super().__call__(args)

        runner = FailingRunner(self.outputs(issues))

        report = release_close_guard.run_guard(
            merge_sha=MERGE_SHA, repo_slug=REPO, dry_run=False, runner=runner
        )

        self.assertEqual([], report.reopened)
        self.assertEqual([(3972, "gh: HTTP 403")], report.failed)
        self.assertEqual(1, release_close_guard.exit_code(report))
        self.assertIn("::warning::", release_close_guard.render_annotations(report))


if __name__ == "__main__":
    unittest.main()
