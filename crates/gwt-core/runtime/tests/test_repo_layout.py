"""Phase 8: tests for --repo-hash / --worktree-hash / --scope path resolution.

These tests will fail until the runner is redesigned to compute db_path internally
from (repo_hash, worktree_hash, scope) instead of accepting --db-path directly.
"""

from __future__ import annotations

import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class ResolveDbPathTests(unittest.TestCase):
    def test_issue_scope_omits_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="issues",
        )
        self.assertTrue(str(path).endswith("/.gwt/index/abc1234567890def/issues"))

    def test_specs_scope_includes_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="specs",
        )
        self.assertTrue(
            str(path).endswith(
                "/.gwt/index/abc1234567890def/worktrees/111122223333ffff/specs"
            )
        )

    def test_files_scope_includes_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="files",
        )
        self.assertTrue(
            str(path).endswith(
                "/.gwt/index/abc1234567890def/worktrees/111122223333ffff/files"
            )
        )

    def test_files_docs_scope(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="files-docs",
        )
        self.assertTrue(
            str(path).endswith(
                "/.gwt/index/abc1234567890def/worktrees/111122223333ffff/files-docs"
            )
        )

    def test_specs_scope_without_worktree_hash_raises(self):
        with self.assertRaises(ValueError):
            runner.resolve_db_path(
                repo_hash="abc1234567890def",
                worktree_hash=None,
                scope="specs",
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


if __name__ == "__main__":
    unittest.main()
