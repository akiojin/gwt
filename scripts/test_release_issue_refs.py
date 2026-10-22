from __future__ import annotations

import importlib.util
import json
import sys
import unittest
import subprocess
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit
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

        self.assertEqual([2000, 2001, 2002], result.delivered_issues)
        self.assertEqual([2003], result.reference_only_issues)
        self.assertEqual(
            [
                "PR #1234 references #2003 only in `Related Issues / Links`; "
                "listed as reference-only."
            ],
            result.warnings,
        )

    def test_parse_pr_body_flags_related_only_refs(self) -> None:
        result = release_issue_refs.parse_pr_body_refs(PR_1585_BODY, pr_number=1585)

        self.assertEqual([], result.delivered_issues)
        self.assertEqual([1457], result.reference_only_issues)
        self.assertEqual(
            [
                "PR #1585 references #1457 only in `Related Issues / Links`; "
                "listed as reference-only."
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

        self.assertEqual([1589], result.delivered_issues)
        self.assertEqual([1457], result.reference_only_issues)
        self.assertEqual(
            [
                "Reference-only issues detected: #1457. They are listed under `Related Issues / Links`.",
                "PR #1585 references #1457 only in `Related Issues / Links`; listed as reference-only.",
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

        self.assertEqual([], result.delivered_issues)
        self.assertEqual([1700], result.reference_only_issues)
        self.assertIn(
            "gwt-spec issues moved to reference-only: #1700. "
            "gwt-spec issues are settled per phase, never by a release.",
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

        self.assertEqual([], result.delivered_issues)
        self.assertEqual([1700], result.reference_only_issues)
        self.assertIn(
            "gwt-spec issues moved to reference-only: #1700. "
            "gwt-spec issues are settled per phase, never by a release.",
            result.warnings,
        )

    def test_non_gwt_spec_issue_stays_delivered(self) -> None:
        """Non-gwt-spec issue stays in delivered."""
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

        self.assertEqual([1589], result.delivered_issues)
        self.assertEqual([], result.reference_only_issues)
        gwt_spec_warnings = [
            w for w in result.warnings if "gwt-spec" in w
        ]
        self.assertEqual([], gwt_spec_warnings)


GITHUB_CLOSING_RE = release_issue_refs.CLOSING_REFERENCE_RE


class ClosingKeywordNeutralizationTests(unittest.TestCase):
    """Issue #3545 AC-2 / AC-4: closing keywords never survive body generation."""

    def test_neutralize_wraps_reference_after_every_keyword_form(self) -> None:
        body = dedent(
            """\
            Closes #3527
            fixes: #3528
            Resolved akiojin/gwt#3529
            closed https://github.com/akiojin/gwt/issues/3530
            - fix(launch): skip legacy resume (#3527 対応)
            """
        )

        result = release_issue_refs.neutralize_closing_keywords(body)

        self.assertEqual(
            dedent(
                """\
                Closes `#3527`
                fixes: `#3528`
                Resolved `akiojin/gwt#3529`
                closed `https://github.com/akiojin/gwt/issues/3530`
                - fix(launch): skip legacy resume (#3527 対応)
                """
            ),
            result,
        )
        self.assertIsNone(GITHUB_CLOSING_RE.search(result))

    def test_neutralize_is_idempotent_and_keeps_plain_refs(self) -> None:
        body = "Related: #3540, see also enclosed #99 and prefix #100"

        once = release_issue_refs.neutralize_closing_keywords(body)

        self.assertEqual(body, once)
        self.assertEqual(once, release_issue_refs.neutralize_closing_keywords(once))

    def test_contains_closing_reference_matches_github_keyword_grammar(self) -> None:
        self.assertTrue(release_issue_refs.contains_closing_reference("Closes #1"))
        self.assertTrue(release_issue_refs.contains_closing_reference("fix:  #1"))
        self.assertFalse(release_issue_refs.contains_closing_reference("Closes `#1`"))
        self.assertFalse(release_issue_refs.contains_closing_reference("hotfix #1"))
        self.assertFalse(release_issue_refs.contains_closing_reference("#1 closes the loop"))


class ReleasePrBodyTests(unittest.TestCase):
    """Issue #3545 AC-1: the generated Release PR body is reference-only."""

    def report(self, delivered, reference_only, warnings=()):
        return release_issue_refs.ReleaseIssueRefs(
            repo="akiojin/gwt",
            range="v9.91.0..HEAD",
            refs=[],
            delivered_issues=list(delivered),
            reference_only_issues=list(reference_only),
            warnings=list(warnings),
        )

    def test_body_lists_delivered_issues_without_closing_keywords(self) -> None:
        body = release_issue_refs.render_release_pr_body(
            self.report([3527, 3528], [3540]), version="v9.92.0", bump="minor"
        )

        self.assertIn("## Summary", body)
        self.assertIn("- v9.92.0 (bump: minor)", body)
        self.assertIn("## Delivered Issues", body)
        self.assertIn("- #3527\n- #3528", body)
        self.assertIn("## Related Issues / Links", body)
        self.assertIn("- #3540", body)
        self.assertNotIn("## Closing Issues", body)
        self.assertNotIn("Closes", body)
        self.assertIsNone(GITHUB_CLOSING_RE.search(body))

    def test_body_says_none_when_no_issues(self) -> None:
        body = release_issue_refs.render_release_pr_body(
            self.report([], []), version="v9.92.0", bump="auto"
        )

        self.assertIn("## Delivered Issues\n\nNone", body)
        self.assertIn("## Related Issues / Links\n\nNone", body)

    def test_body_neutralizes_closing_keywords_in_free_text(self) -> None:
        body = release_issue_refs.render_release_pr_body(
            self.report([1], []),
            version="v9.92.0",
            bump="patch",
            notes="### Bug Fixes\n\n- **launch:** Skip legacy resume, fixes #3527\n",
        )

        self.assertIn("fixes `#3527`", body)
        self.assertIsNone(GITHUB_CLOSING_RE.search(body))

    def test_body_lists_breaking_commits_without_bumping(self) -> None:
        # Issue #4373 AC-2: breaking markers are reported in the Release PR
        # body even though they no longer raise the bump level.
        body = release_issue_refs.render_release_pr_body(
            self.report([1], []),
            version="v9.98.0",
            bump="auto",
            breaking_commits=[
                "aaa1111 feat!: drop old API",
                "bbb2222 fix(verification): closes #3527",
            ],
        )

        self.assertIn("## Breaking Changes", body)
        self.assertIn("- aaa1111 feat!: drop old API", body)
        self.assertIn("- bbb2222 fix(verification): closes `#3527`", body)
        self.assertIn("not applied to the version", body)
        self.assertIsNone(GITHUB_CLOSING_RE.search(body))

    def test_body_omits_breaking_section_when_none(self) -> None:
        body = release_issue_refs.render_release_pr_body(
            self.report([1], []), version="v9.98.0", bump="auto", breaking_commits=[]
        )

        self.assertNotIn("## Breaking Changes", body)

    def test_render_text_has_no_closing_keywords(self) -> None:
        text = release_issue_refs.render_text(self.report([3527], [3540], ["w"]))

        self.assertIn("Delivered issues:", text)
        self.assertIn("- #3527", text)
        self.assertNotIn("Closes", text)
        self.assertIsNone(GITHUB_CLOSING_RE.search(text))


class FeedbackApiRunner:
    """No command is executed; every API response has its real provenance shape."""
    def __init__(self):
        self.calls = []
        self.posts = []
        self.issues = {
            20: self.issue(20, "akiojin", "<!-- gwt-feedback-source:10 -->"),
            10: self.issue(10, "external", "<!-- gwt-feedback:11111111-1111-1111-1111-111111111111 -->"),
            99: self.issue(99, "akiojin", "Unrelated requirements"),
        }
        self.comments = {20: [], 10: []}
        self.release = {"draft": False, "published_at": "2026-10-22T00:00:00Z",
                        "tag_name": "v1.2.3", "html_url": "https://github.com/akiojin/gwt/releases/tag/v1.2.3"}
        self.fail_page = None
        self.lose_post = False
        self.persist_post = True
        self.merged = True
        self.merge_sha = "a" * 40
        self.pr_author = "akiojin"

    @staticmethod
    def issue(number, author, body="", labels=()):
        return {"number": number, "user": {"login": author}, "body": body,
                "labels": [{"name": name} for name in labels]}

    @staticmethod
    def comment(number, author, body):
        return {"id": number, "user": {"login": author}, "body": body}

    def __call__(self, args):
        args = list(args)
        self.calls.append(args)
        if args[:2] == ["git", "rev-list"]:
            return "a" * 40 + "\n"
        if args[:6] != ["gh", "api", "--hostname", "github.com", "--method", "GET"] and args[:6] != ["gh", "api", "--hostname", "github.com", "--method", "POST"]:
            raise AssertionError(f"Unexpected/non-fixed command: {args!r}")
        path = args[6]
        if path == "repos/akiojin/gwt/releases/tags/v1.2.3":
            return json.dumps(self.release)
        if path.startswith("repos/akiojin/gwt/pulls/"):
            return json.dumps({"user": {"login": self.pr_author},
                               "merged_at": "2026-10-21T00:00:00Z" if self.merged else None,
                               "merge_commit_sha": self.merge_sha})
        parsed = urlsplit(path)
        parts = parsed.path.split("/")
        if parts[:4] != ["repos", "akiojin", "gwt", "issues"]:
            raise AssertionError(path)
        number = int(parts[4])
        if len(parts) == 5:
            return json.dumps(self.issues[number])
        if parts[5] != "comments":
            raise AssertionError(path)
        if args[5] == "POST":
            body = args[args.index("-f") + 1].removeprefix("body=")
            self.posts.append((number, body))
            comment = self.comment(10000 + len(self.posts), "github-actions[bot]", body)
            if self.persist_post:
                self.comments.setdefault(number, []).append(comment)
            if self.lose_post:
                raise subprocess.CalledProcessError(1, args, stderr="response lost")
            return json.dumps(comment)
        page = int(parse_qs(parsed.query)["page"][0])
        if self.fail_page == (number, page):
            raise subprocess.CalledProcessError(1, args, stderr="page unavailable")
        return json.dumps(self.comments.get(number, [])[(page-1)*100:page*100])


class FeedbackReleaseNotificationTests(unittest.TestCase):
    def report(self):
        return release_issue_refs.ReleaseIssueRefs(
            repo="akiojin/gwt", range="v1.2.2..v1.2.3",
            refs=[release_issue_refs.CommitRef(100, "pr", "squash", [20], [99])],
            delivered_issues=[20], reference_only_issues=[99])

    def notify(self, api, report=None):
        return release_issue_refs.notify_feedback_results(report or self.report(), "v1.2.3", runner=api)

    def test_notify_mode_is_explicit(self):
        self.assertIn("--notify-feedback", release_issue_refs.build_parser().format_help())

    def test_owner_body_recovers_source_and_ignores_forged_links_and_dedup(self):
        api = FeedbackApiRunner()
        api.comments[20] = [api.comment(1, "external", "<!-- gwt-feedback-owner:999:20 -->")]
        api.comments[10] = [
            api.comment(2, "external", "<!-- gwt-feedback-release:10:20:v1.2.3 -->"),
            api.comment(3, "akiojin", "```text\n<!-- gwt-feedback-release:10:20:v1.2.3 -->\n```"),
        ]
        self.notify(api)
        self.assertEqual([10], [number for number, _ in api.posts])
        body = api.posts[0][1]
        for link in ("/pull/100", "/issues/20", "/releases/tag/v1.2.3"):
            self.assertIn("https://github.com/akiojin/gwt" + link, body)
        self.assertIn("<!-- gwt-feedback-release:10:20:v1.2.3 -->", body)
        self.notify(api)
        self.assertEqual(1, len(api.posts), "a trusted posted marker makes rerun idempotent")

    def test_trusted_comment_uses_its_own_author_and_exact_owner(self):
        api = FeedbackApiRunner()
        api.issues[20]["body"] = "Curated requirements"
        api.comments[20] = [
            api.comment(1, "external", "<!-- gwt-feedback-owner:999:20 -->"),
            api.comment(2, "github-actions[bot]", "<!-- gwt-feedback-owner:999:21 -->"),
            api.comment(3, "akiojin", "```text\n<!-- gwt-feedback-owner:10:20 -->\n```"),
        ]
        self.notify(api)
        self.assertEqual([], api.posts, "quoted reporter evidence is not a dedicated linkage comment")
        api.comments[20][-1]["body"] = "<!-- gwt-feedback-owner:10:20 -->\n\nCurated source link"
        self.notify(api)
        self.assertEqual([10], [number for number, _ in api.posts])

    def test_direct_aggregate_owner_receives_notice_without_raw_reclassification(self):
        api = FeedbackApiRunner()
        api.issues[20]["body"] = "Curated requirements"
        api.comments[20] = [api.comment(1, "external", "<!-- gwt-feedback:11111111-1111-1111-1111-111111111111 -->\nnew environment")]
        self.notify(api)
        self.assertEqual([20], [number for number, _ in api.posts])

    def test_untrusted_original_owner_or_raw_owner_never_supplies_authority(self):
        for author, body, labels in [
            ("external", "<!-- gwt-feedback-source:10 -->", []),
            ("akiojin", "<!-- gwt-feedback:11111111-1111-1111-1111-111111111111 -->", []),
            ("akiojin", "<!-- gwt-feedback-source:10 -->", ["gwt-feedback"]),
        ]:
            with self.subTest(author=author, body=body):
                api = FeedbackApiRunner()
                api.issues[20] = api.issue(20, author, body, labels)
                self.notify(api)
                self.assertEqual([], api.posts)

    def test_external_pr_requires_trusted_exact_leading_owner_settlement(self):
        api = FeedbackApiRunner()
        api.pr_author = "external-contributor"
        api.issues[20]["labels"] = [{"name": "gwt-spec"}]
        marker = f"<!-- gwt-merged-issue-settlement v1 pr=100 sha={api.merge_sha} -->"
        api.comments[20] = [api.comment(1, "external-contributor", marker)]
        self.notify(api)
        self.assertEqual([], api.posts, "merged body refs and reporter-forged proof grant no linkage")
        api.comments[20] = [api.comment(1, "github-actions[bot]", marker.replace(api.merge_sha, "b" * 40))]
        self.notify(api)
        self.assertEqual([], api.posts, "a settlement for another merge SHA grants no linkage")
        api.comments[20] = [api.comment(1, "akiojin", "Quoted proof:\n" + marker)]
        self.notify(api)
        self.assertEqual([], api.posts, "the exact proof must lead the trusted owner comment")
        api.comments[20] = [api.comment(1, "github-actions[bot]", marker + "\n\nDelivered phase")]
        self.notify(api)
        self.assertEqual([10], [number for number, _ in api.posts])
        self.assertIn("/pull/100", api.posts[0][1])
        self.assertIn("SPEC phase", api.posts[0][1])
        self.assertIn("does not mark", api.posts[0][1])

    def test_any_raw_feedback_prefix_excludes_an_execution_owner(self):
        api = FeedbackApiRunner()
        api.issues[20]["body"] += "\n<!-- GWT-Feedback:legacy-or-incomplete"
        self.notify(api)
        self.assertEqual([], api.posts, "raw prefix excludes even a trusted original owner")

    def test_incomplete_pagination_and_unknown_comment_author_hold_notifications(self):
        api = FeedbackApiRunner()
        api.comments[10] = [api.comment(i, "external", "context") for i in range(1, 101)]
        api.fail_page = (10, 2)
        with self.assertRaisesRegex(RuntimeError, "pending|unverified"):
            self.notify(api)
        self.assertEqual([], api.posts)
        api.fail_page = None
        api.comments[10] = [{"id": 1, "body": "<!-- gwt-feedback-release:10:20:v1.2.3 -->"}]
        with self.assertRaisesRegex(ValueError, "author"):
            self.notify(api)
        self.assertEqual([], api.posts)

    def test_later_page_trusted_notification_suppresses_post(self):
        api = FeedbackApiRunner()
        api.comments[10] = [api.comment(i, "external", "context") for i in range(1, 101)]
        api.comments[10].append(api.comment(101, "akiojin", "<!-- gwt-feedback-release:10:20:v1.2.3 -->"))
        self.notify(api)
        self.assertEqual([], api.posts)
        self.assertTrue(any("page=2" in arg for call in api.calls for arg in call))

    def test_publication_and_fixed_repository_are_required_before_mutation(self):
        api = FeedbackApiRunner()
        api.release["draft"] = True
        with self.assertRaisesRegex(ValueError, "published"):
            self.notify(api)
        self.assertEqual([], api.posts)
        api.release["draft"] = False
        api.release["published_at"] = None
        with self.assertRaisesRegex(ValueError, "published"):
            self.notify(api)
        report = self.report()
        report.repo = "other/project"
        with self.assertRaisesRegex(ValueError, "akiojin/gwt"):
            self.notify(api, report)
        self.assertEqual([], api.posts)

    def test_delivered_spec_phase_is_not_lost_in_global_reference_only_filter(self):
        api = FeedbackApiRunner()
        # Actual partial-SPEC PR shape: leave the parent open and list prior PRs.
        body = """## Closing Issues

None

## Related Issues / Links

- Refs #4810。親 Issue は閉じません。#5883 と #5900 の配信済み境界を前提に、残る受付部分を追加します。
"""
        parsed = release_issue_refs.parse_pr_body_refs(body, pr_number=100)
        self.assertEqual([], parsed.delivered_issues)
        self.assertEqual([4810, 5883, 5900], parsed.reference_only_issues)
        report = self.report()
        report.refs = [release_issue_refs.CommitRef(100, "pr", "squash",
                       parsed.delivered_issues, parsed.reference_only_issues)]
        report.delivered_issues = []
        report.reference_only_issues = parsed.reference_only_issues
        api.issues[4810] = api.issue(4810, "akiojin", "<!-- gwt-feedback-source:10 -->", ["gwt-spec"])
        api.comments[4810] = []
        for number in (5883, 5900):
            api.issues[number] = {"number": number, "pull_request": {"url": f"https://api.github.com/repos/akiojin/gwt/pulls/{number}"}}
        self.notify(api, report)
        self.assertEqual([], api.posts, "trusted PR authorship does not authorize Related-only refs")
        marker = f"<!-- gwt-merged-issue-settlement v1 pr=100 sha={api.merge_sha} -->"
        api.comments[4810] = [api.comment(1, "external", marker)]
        self.notify(api, report)
        self.assertEqual([], api.posts, "a reporter-forged settlement is not phase delivery")
        api.comments[4810] = [api.comment(1, "github-actions[bot]", marker + "\n\nDelivered phase")]
        self.notify(api, report)
        self.assertEqual([10], [number for number, _ in api.posts])
        self.assertIn("/issues/4810", api.posts[0][1])
        self.assertIn("SPEC phase", api.posts[0][1])
        self.assertIn("does not mark", api.posts[0][1])
        self.assertNotIn("/pull/5883", api.posts[0][1])
        self.assertNotIn("/pull/5900", api.posts[0][1])

    def test_unmerged_or_out_of_range_pr_does_not_claim_delivery(self):
        api = FeedbackApiRunner()
        api.merged = False
        with self.assertRaisesRegex(ValueError, "merged"):
            self.notify(api)
        self.assertEqual([], api.posts)
        api.merged = True
        api.merge_sha = "b" * 40
        with self.assertRaisesRegex(ValueError, "merged"):
            self.notify(api)
        self.assertEqual([], api.posts)

    def test_ambiguous_post_reconciles_and_never_blindly_reposts(self):
        api = FeedbackApiRunner()
        api.lose_post = True
        self.notify(api)
        self.assertEqual(1, len(api.posts))
        self.notify(api)
        self.assertEqual(1, len(api.posts))
        api = FeedbackApiRunner()
        api.lose_post = True
        api.persist_post = False
        with self.assertRaisesRegex(RuntimeError, "pending|unverified"):
            self.notify(api)
        self.assertEqual(1, len(api.posts), "unknown POST is not retried in this invocation")

    def test_release_range_skips_draft_tags_and_nonancestor_releases(self):
        outputs = {
            ("gh", "api", "--hostname", "github.com", "--method", "GET", "repos/akiojin/gwt/releases?per_page=100&page=1"): json.dumps([
                {"id": 4, "tag_name": "v1.2.3", "draft": False, "published_at": "now"},
                {"id": 3, "tag_name": "v1.2.2", "draft": True, "published_at": None},
                {"id": 2, "tag_name": "v2.0.0", "draft": False, "published_at": "earlier"},
                {"id": 1, "tag_name": "v1.2.1", "draft": False, "published_at": "earlier"}]),
            ("git", "tag", "--merged", "v1.2.3"): "v1.2.1\nv1.2.2\nv1.2.3\n",
            ("git", "describe", "--tags", "--abbrev=0", "--match", "v1.2.1", "v1.2.3"): "v1.2.1\n",
        }
        self.assertEqual("v1.2.1..v1.2.3", release_issue_refs.feedback_release_range("v1.2.3", FakeRunner(outputs)))

    def test_release_range_requires_complete_release_pagination(self):
        calls = []
        def runner(args):
            calls.append(list(args))
            if args[-1].endswith("page=1"):
                return json.dumps([{"id": i, "draft": True} for i in range(1, 101)])
            raise subprocess.CalledProcessError(1, args, stderr="second page unavailable")
        with self.assertRaises(subprocess.CalledProcessError):
            release_issue_refs.feedback_release_range("v1.2.3", runner)
        self.assertEqual(2, len(calls))
        self.assertFalse(any(call[0] == "git" for call in calls))

    def test_collection_host_is_fixed_for_api_and_pr_reads(self):
        calls = []
        with patch.object(release_issue_refs, "run_command", side_effect=lambda args: calls.append(args) or "{}"):
            release_issue_refs._fixed_github_collection_command(["gh", "api", "repos/akiojin/gwt/issues/20"])
            release_issue_refs._fixed_github_collection_command(["gh", "pr", "view", "100", "--repo", "akiojin/gwt", "--json", "body"])
        self.assertEqual(["gh", "api", "--hostname", "github.com"], calls[0][:4])
        self.assertEqual("https://github.com/akiojin/gwt", calls[1][calls[1].index("--repo") + 1])

    def test_release_workflow_notifies_only_after_publish_with_full_history(self):
        workflow = MODULE_PATH.parents[1].joinpath(".github/workflows/release.yml").read_text()
        upload = workflow.split("  upload-release:", 1)[1]
        self.assertIn("fetch-depth: 0", upload)
        self.assertLess(upload.index("--draft=false"), upload.index("--notify-feedback"))
        self.assertIn("--repo akiojin/gwt", upload)
        self.assertIn("--version", upload)
        self.assertIn('--range "$TAG_NAME"', upload)
        self.assertIn("if: github.repository == 'akiojin/gwt'", upload)


if __name__ == "__main__":
    unittest.main()
