from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path
from textwrap import dedent

MODULE_PATH = Path(__file__).with_name("release_issue_refs.py")
MODULE_SPEC = importlib.util.spec_from_file_location("release_issue_refs", MODULE_PATH)
assert MODULE_SPEC is not None
assert MODULE_SPEC.loader is not None
release_issue_refs = importlib.util.module_from_spec(MODULE_SPEC)
sys.modules[MODULE_SPEC.name] = release_issue_refs
MODULE_SPEC.loader.exec_module(release_issue_refs)

PR_1585_BODY = dedent(
    """\
    ## Summary

    - Refit and refresh the terminal when focus or visibility returns so Windows terminal corruption can recover without manual tab switching.

    ## Closing Issues

    None

    ## Related Issues / Links

    - #1457
    """
)


class FakeRunner:
    def __init__(self, outputs: dict[tuple[str, ...], str]) -> None:
        self.outputs = outputs

    def __call__(self, args: list[str] | tuple[str, ...]) -> str:
        key = tuple(args)
        if key not in self.outputs:
            raise AssertionError(f"Unexpected command: {key!r}")
        return self.outputs[key]


class ReleaseIssueRefsTests(unittest.TestCase):
    def test_parse_pr_body_collects_keywords_and_closing_section(self) -> None:
        body = dedent(
            """\
            ## Summary

            This change resolves #2000 and keeps release notes aligned.

            ## Closing Issues

            #2001
            Closes #2002

            ## Related Issues / Links

            - #2003
            - https://example.com/task
            """
        )

        result = release_issue_refs.parse_pr_body_refs(body, pr_number=1234)

        self.assertEqual([2000, 2001, 2002], result.auto_close_issues)
        self.assertEqual([2003], result.reference_only_issues)
        self.assertEqual(
            [
                "PR #1234 references #2003 only in `Related Issues / Links`; "
                "they will not auto-close on release."
            ],
            result.warnings,
        )

    def test_parse_pr_body_flags_related_only_refs(self) -> None:
        result = release_issue_refs.parse_pr_body_refs(PR_1585_BODY, pr_number=1585)

        self.assertEqual([], result.auto_close_issues)
        self.assertEqual([1457], result.reference_only_issues)
        self.assertEqual(
            [
                "PR #1585 references #1457 only in `Related Issues / Links`; "
                "they will not auto-close on release."
            ],
            result.warnings,
        )

    def test_collect_release_issue_refs_keeps_reference_only_issues_visible(self) -> None:
        runner = FakeRunner(
            {
                (
                    "git",
                    "log",
                    "--pretty=%s",
                    "--no-merges",
                    "v8.6.0..HEAD",
                ): (
                    "fix(gui): refresh terminal when focus returns (#1585)\n"
                    "fix: hook error (#1589)\n"
                ),
                ("git", "log", "--merges", "--pretty=%s", "v8.6.0..HEAD"): "",
                ("gh", "api", "repos/akiojin/gwt/issues/1585"): '{"pull_request":{"url":"https://example.com/pr/1585"}}',
                (
                    "gh",
                    "pr",
                    "view",
                    "1585",
                    "--repo",
                    "akiojin/gwt",
                    "--json",
                    "body",
                ): json.dumps({"body": PR_1585_BODY}),
                ("gh", "api", "repos/akiojin/gwt/issues/1589"): "{}",
            }
        )

        result = release_issue_refs.collect_release_issue_refs(
            "v8.6.0..HEAD",
            repo_slug="akiojin/gwt",
            runner=runner,
        )

        self.assertEqual([1589], result.auto_close_issues)
        self.assertEqual([1457], result.reference_only_issues)
        self.assertEqual(
            [
                "Reference-only issues detected: #1457. Add them to `Closing Issues` if they should auto-close.",
                "PR #1585 references #1457 only in `Related Issues / Links`; they will not auto-close on release.",
            ],
            result.warnings,
        )
        self.assertEqual(["pr", "issue"], [ref.kind for ref in result.refs])

    def test_gwt_spec_direct_issue_moved_to_reference_only(self) -> None:
        """Direct issue ref with gwt-spec label moves to reference_only."""
        runner = FakeRunner(
            {
                (
                    "git",
                    "log",
                    "--pretty=%s",
                    "--no-merges",
                    "v8.14.0..HEAD",
                ): "feat(spec): adopt artifact-first issue workflow (#1700)\n",
                ("git", "log", "--merges", "--pretty=%s", "v8.14.0..HEAD"): "",
                ("gh", "api", "repos/akiojin/gwt/issues/1700"): json.dumps(
                    {"labels": [{"name": "gwt-spec"}, {"name": "enhancement"}]}
                ),
            }
        )

        result = release_issue_refs.collect_release_issue_refs(
            "v8.14.0..HEAD",
            repo_slug="akiojin/gwt",
            runner=runner,
        )

        self.assertEqual([], result.auto_close_issues)
        self.assertEqual([1700], result.reference_only_issues)
        self.assertIn(
            "gwt-spec issues moved to reference-only: #1700. "
            "gwt-spec issues are never auto-closed by releases.",
            result.warnings,
        )

    def test_gwt_spec_issue_via_pr_closing_moved_to_reference_only(self) -> None:
        """PR body `Closes #N` where N has gwt-spec label moves N to reference_only."""
        pr_body = dedent(
            """\
            ## Closing Issues

            #1700

            ## Related Issues / Links

            None"""
        )
        runner = FakeRunner(
            {
                (
                    "git",
                    "log",
                    "--pretty=%s",
                    "--no-merges",
                    "v8.14.0..HEAD",
                ): "fix(spec): harden issue migration retries (#1701)\n",
                ("git", "log", "--merges", "--pretty=%s", "v8.14.0..HEAD"): "",
                ("gh", "api", "repos/akiojin/gwt/issues/1701"): json.dumps(
                    {"pull_request": {"url": "https://example.com/pr/1701"}}
                ),
                (
                    "gh",
                    "pr",
                    "view",
                    "1701",
                    "--repo",
                    "akiojin/gwt",
                    "--json",
                    "body",
                ): json.dumps({"body": pr_body}),
                ("gh", "api", "repos/akiojin/gwt/issues/1700"): json.dumps(
                    {"labels": [{"name": "gwt-spec"}]}
                ),
            }
        )

        result = release_issue_refs.collect_release_issue_refs(
            "v8.14.0..HEAD",
            repo_slug="akiojin/gwt",
            runner=runner,
        )

        self.assertEqual([], result.auto_close_issues)
        self.assertEqual([1700], result.reference_only_issues)
        self.assertIn(
            "gwt-spec issues moved to reference-only: #1700. "
            "gwt-spec issues are never auto-closed by releases.",
            result.warnings,
        )

    def test_non_gwt_spec_issue_stays_in_auto_close(self) -> None:
        """Non-gwt-spec issue stays in auto_close."""
        runner = FakeRunner(
            {
                (
                    "git",
                    "log",
                    "--pretty=%s",
                    "--no-merges",
                    "v8.14.0..HEAD",
                ): "fix: hook error (#1589)\n",
                ("git", "log", "--merges", "--pretty=%s", "v8.14.0..HEAD"): "",
                ("gh", "api", "repos/akiojin/gwt/issues/1589"): json.dumps(
                    {"labels": [{"name": "bug"}]}
                ),
            }
        )

        result = release_issue_refs.collect_release_issue_refs(
            "v8.14.0..HEAD",
            repo_slug="akiojin/gwt",
            runner=runner,
        )

        self.assertEqual([1589], result.auto_close_issues)
        self.assertEqual([], result.reference_only_issues)
        gwt_spec_warnings = [
            w for w in result.warnings if "gwt-spec" in w
        ]
        self.assertEqual([], gwt_spec_warnings)


if __name__ == "__main__":
    unittest.main()
