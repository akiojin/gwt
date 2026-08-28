"""Phase 8: tests for --repo-hash / --worktree-hash / --scope path resolution.

These tests will fail until the runner is redesigned to compute db_path internally
from (repo_hash, worktree_hash, scope) instead of accepting --db-path directly.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class ResolveDbPathTests(unittest.TestCase):
    def test_file_index_v2_root_is_additive_and_repo_scoped(self):
        resolver = getattr(runner, "resolve_file_index_v2_root", None)
        self.assertIsNotNone(
            resolver,
            "Phase 71 requires the additive resolve_file_index_v2_root API",
        )

        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index"
            actual = resolver("abc1234567890def", db_root=db_root)
            self.assertEqual(
                actual,
                db_root / "abc1234567890def" / "file-index-v2",
            )

    def test_issue_scope_omits_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="issues",
        )
        expected = (".gwt", "index", "abc1234567890def", "issues")
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_specs_scope_is_repo_scoped(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="specs",
        )
        expected = (
            ".gwt",
            "index",
            "abc1234567890def",
            "specs",
        )
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_files_scope_includes_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="files",
        )
        expected = (
            ".gwt",
            "index",
            "abc1234567890def",
            "worktrees",
            "111122223333ffff",
            "files",
        )
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_files_docs_scope(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="files-docs",
        )
        expected = (
            ".gwt",
            "index",
            "abc1234567890def",
            "worktrees",
            "111122223333ffff",
            "files-docs",
        )
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_memory_scope_is_repo_scoped(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="memory",
        )
        expected = (".gwt", "index", "abc1234567890def", "memory")
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_memory_scope_ignores_worktree_hash(self):
        with_wt = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="memory",
        )
        without_wt = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="memory",
        )
        self.assertEqual(with_wt, without_wt)
        self.assertNotIn("worktrees", with_wt.parts)

    def test_files_scope_without_worktree_hash_raises(self):
        with self.assertRaises(ValueError):
            runner.resolve_db_path(
                repo_hash="abc1234567890def",
                worktree_hash=None,
                scope="files",
            )

    def test_unknown_scope_raises(self):
        with self.assertRaises(ValueError):
            runner.resolve_db_path(
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                scope="bogus",
            )


class CliArgumentTests(unittest.TestCase):
    """The argparse parser must accept the new flags."""

    def test_parse_args_accepts_repo_hash(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--query",
                "hello",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.repo_hash, "abc1234567890def")
            self.assertEqual(args.worktree_hash, "111122223333ffff")

    def test_parse_args_accepts_scope(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--scope",
                "files-docs",
                "--query",
                "x",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.scope, "files-docs")

    def test_parse_args_accepts_no_auto_build(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--query",
                "x",
                "--no-auto-build",
            ],
        ):
            args = runner.parse_args()
            self.assertTrue(args.no_auto_build)

    def test_parse_args_accepts_search_multi_scopes(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-multi",
                "--repo-hash",
                "abc1234567890def",
                "--query",
                "Git",
                "--scopes",
                "issues,specs,board,memory",
                "--no-auto-build",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.action, "search-multi")
            self.assertEqual(args.scopes, "issues,specs,board,memory")
            self.assertTrue(args.no_auto_build)

    def test_parse_args_accepts_respect_ttl(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-issues",
                "--repo-hash",
                "abc1234567890def",
                "--project-root",
                "/tmp/proj",
                "--respect-ttl",
            ],
        ):
            args = runner.parse_args()
            self.assertTrue(args.respect_ttl)

    def test_parse_args_accepts_mode(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--project-root",
                "/tmp/proj",
                "--mode",
                "incremental",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.mode, "incremental")

    def test_file_index_protocol_defaults_to_legacy(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--project-root",
                "/tmp/proj",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(
                getattr(args, "file_index_protocol", None),
                "legacy",
                "existing actions must keep legacy semantics unless v2 is explicit",
            )

    def test_parse_args_accepts_explicit_file_index_v2_protocol(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--project-root",
                "/tmp/proj",
                "--file-index-protocol",
                "v2",
            ],
        ):
            try:
                args = runner.parse_args()
            except SystemExit as error:
                self.fail(
                    "Phase 71 explicit --file-index-protocol v2 is unsupported: "
                    f"{error}"
                )
            self.assertEqual(args.file_index_protocol, "v2")


class ProbeCapabilityTests(unittest.TestCase):
    def test_probe_advertises_additive_file_index_protocols(self):
        result = runner.action_probe()
        self.assertTrue(result.get("ok"), result)
        self.assertEqual(
            result.get("file_index_protocols"),
            ["legacy", "v2"],
            "probe must preserve legacy while advertising explicit file index v2",
        )


if __name__ == "__main__":
    unittest.main()
