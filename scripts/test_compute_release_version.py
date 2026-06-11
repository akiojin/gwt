#!/usr/bin/env python3
"""Unit tests for scripts/compute_release_version.py."""

from __future__ import annotations

import unittest

import compute_release_version as crv


class ParseTagTests(unittest.TestCase):
    def test_parses_semver_tag(self):
        self.assertEqual(crv.parse_tag("v9.54.0"), (9, 54, 0))

    def test_ignores_non_semver(self):
        self.assertIsNone(crv.parse_tag("v9.54"))
        self.assertIsNone(crv.parse_tag("release-9.54.0"))
        self.assertIsNone(crv.parse_tag(None))


class ClassifyBumpTests(unittest.TestCase):
    def test_breaking_subject_marker_is_major(self):
        self.assertEqual(crv.classify_bump(["feat!: drop old API"], "feat!: drop old API\n"), "major")

    def test_breaking_body_token_is_major(self):
        body = "feat: thing\n\nBREAKING CHANGE: removes X\n"
        self.assertEqual(crv.classify_bump(["feat: thing"], body), "major")

    def test_feat_is_minor(self):
        self.assertEqual(crv.classify_bump(["feat(ui): add panel", "fix: typo"], "feat(ui): add panel\nfix: typo\n"), "minor")

    def test_fix_only_is_patch(self):
        self.assertEqual(crv.classify_bump(["fix: bug", "docs: note"], "fix: bug\ndocs: note\n"), "patch")

    def test_docs_chore_only_is_patch(self):
        self.assertEqual(crv.classify_bump(["docs: x", "chore: y"], "docs: x\nchore: y\n"), "patch")

    def test_breaking_prose_in_body_is_not_major(self):
        # "BREAKING CHANGE" appearing mid-line as prose must NOT trigger major.
        body = "feat: doc\n\nThis change documents BREAKING CHANGE behavior in the guide.\n"
        self.assertEqual(crv.classify_bump(["feat: doc"], body), "minor")

    def test_breaking_change_hyphen_footer_is_major(self):
        body = "fix: thing\n\nBREAKING-CHANGE: drops Y\n"
        self.assertEqual(crv.classify_bump(["fix: thing"], body), "major")


class NextVersionTests(unittest.TestCase):
    def test_major(self):
        self.assertEqual(crv.next_version((9, 54, 0), "major"), "10.0.0")

    def test_minor(self):
        self.assertEqual(crv.next_version((9, 54, 0), "minor"), "9.55.0")

    def test_patch(self):
        self.assertEqual(crv.next_version((9, 54, 0), "patch"), "9.54.1")


class ResolveVersionTests(unittest.TestCase):
    def test_auto_minor_from_feat(self):
        version = crv.resolve_version("v9.54.0", ["feat: x"], "feat: x\n", "auto")
        self.assertEqual(version, "9.55.0")

    def test_auto_patch_from_fix(self):
        version = crv.resolve_version("v9.54.0", ["fix: x"], "fix: x\n", "auto")
        self.assertEqual(version, "9.54.1")

    def test_auto_refuses_major(self):
        with self.assertRaises(crv.ReleaseVersionError):
            crv.resolve_version("v9.54.0", ["feat!: x"], "feat!: x\n", "auto")

    def test_explicit_major_allowed(self):
        version = crv.resolve_version("v9.54.0", ["feat!: x"], "feat!: x\n", "major")
        self.assertEqual(version, "10.0.0")

    def test_explicit_override_beats_classification(self):
        # Only fixes present, but operator forces minor.
        self.assertEqual(crv.resolve_version("v9.54.0", ["fix: x"], "fix: x\n", "minor"), "9.55.0")

    def test_no_tag_initial_release(self):
        self.assertEqual(crv.resolve_version(None, ["feat: x"], "feat: x\n", "auto"), "0.1.0")

    def test_invalid_bump_rejected(self):
        with self.assertRaises(crv.ReleaseVersionError):
            crv.resolve_version("v9.54.0", [], "", "nonsense")


class GitWiringTests(unittest.TestCase):
    def test_latest_version_tag_picks_first_semver(self):
        calls: list[list[str]] = []

        def runner(cmd):
            calls.append(list(cmd))
            return "v9.54.0\nv9.53.0\nweird-tag\n"

        self.assertEqual(crv.latest_version_tag(runner), "v9.54.0")
        self.assertEqual(calls[0][:3], ["git", "tag", "--list"])

    def test_compute_uses_injected_runner_end_to_end(self):
        def runner(cmd):
            if cmd[:3] == ["git", "tag", "--list"]:
                return "v9.54.0\n"
            if cmd[:3] == ["git", "rev-list", "--count"]:
                return "2\n"
            return "feat: new thing\nfix: a bug\n"

        self.assertEqual(crv.compute("auto", runner=runner), "9.55.0")

    def test_compute_zero_commits_raises(self):
        def runner(cmd):
            if cmd[:3] == ["git", "tag", "--list"]:
                return "v9.54.0\n"
            if cmd[:3] == ["git", "rev-list", "--count"]:
                return "0\n"
            return ""

        with self.assertRaises(crv.ReleaseVersionError):
            crv.compute("auto", runner=runner)

    def test_gather_commits_excludes_merges_from_both_queries(self):
        calls: list[list[str]] = []

        def runner(cmd):
            calls.append(list(cmd))
            return "feat: x\n"

        crv.gather_commits("v1..HEAD", runner)
        # Both the subject query and the body query must pass --no-merges.
        self.assertTrue(all("--no-merges" in c for c in calls))
        self.assertEqual(len(calls), 2)


if __name__ == "__main__":
    unittest.main()
