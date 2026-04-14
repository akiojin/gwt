"""Phase 8: tests for the search-* auto-build fallback.

When a search action is invoked against a missing index, the runner must
build the index in-process (full mode) and then run the search. The
--no-auto-build flag suppresses this behavior.
"""

from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class AutoBuildFallbackTests(unittest.TestCase):
    def _make_repo(self, root: Path) -> None:
        (root / "src").mkdir(parents=True)
        (root / "src" / "watcher.rs").write_text(
            "//! file system watcher with debounce\n"
            "fn debounce_events() {}\n"
        )
        (root / "README.md").write_text("# project\n")

    def _write_cached_issue(
        self,
        cache_root: Path,
        number: int,
        title: str,
        body: str,
        labels,
    ) -> None:
        issue = cache_root / str(number)
        issue.mkdir(parents=True, exist_ok=True)
        (issue / "meta.json").write_text(
            json.dumps(
                {
                    "number": number,
                    "title": title,
                    "labels": labels,
                    "state": "open",
                    "updated_at": "2026-04-14T00:00:00Z",
                    "comment_ids": [],
                }
            )
        )
        (issue / "body.md").write_text(body)
        sections = issue / "sections"
        sections.mkdir(exist_ok=True)
        (sections / "spec.md").write_text(body)

    def test_search_files_auto_builds_when_index_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            result = runner.action_search_v2(
                action="search-files",
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                project_root=str(root),
                query="watcher",
                n_results=5,
                no_auto_build=False,
                db_root=db_root,
            )

            self.assertTrue(result["ok"], result)
            self.assertIn("results", result)
            db = (
                db_root
                / "abc1234567890def"
                / "worktrees"
                / "111122223333ffff"
                / "files"
            )
            self.assertTrue(db.exists(), f"index dir was not created: {db}")

    def test_search_returns_index_missing_when_no_auto_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"

            result = runner.action_search_v2(
                action="search-files",
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                project_root=str(Path(tmp) / "repo"),
                query="anything",
                n_results=5,
                no_auto_build=True,
                db_root=db_root,
            )

            self.assertFalse(result["ok"])
            self.assertEqual(result.get("error_code"), "INDEX_MISSING")

    def test_progress_emitted_on_stderr(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            buf = io.StringIO()
            with mock.patch.object(sys, "stderr", buf):
                runner.action_search_v2(
                    action="search-files",
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root=str(root),
                    query="watcher",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            stderr_content = buf.getvalue()
            self.assertIn("phase", stderr_content)
            # At least one valid NDJSON line that mentions "indexing".
            saw_indexing_phase = False
            for line in stderr_content.splitlines():
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if obj.get("phase") in ("indexing", "embedding", "writing", "complete"):
                    saw_indexing_phase = True
                    break
            self.assertTrue(
                saw_indexing_phase,
                f"expected NDJSON progress on stderr, got: {stderr_content!r}",
            )

    def test_search_specs_auto_builds_when_index_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(
                cache_root,
                1939,
                "gwt-spec: Semantic search platform",
                "# Semantic search platform\nWatcher debounce semantics.\n",
                ["gwt-spec", "phase/review"],
            )
            self._write_cached_issue(
                cache_root,
                2000,
                "Plain issue",
                "# Plain issue\nWatcher noise that must not appear in spec search.\n",
                ["bug"],
            )

            db_root = Path(tmp) / "index_root"
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_search_v2(
                    action="search-specs",
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertTrue(result["ok"], result)
            self.assertIn("specResults", result)
            self.assertEqual(len(result["specResults"]), 1, result["specResults"])
            self.assertEqual(result["specResults"][0]["spec_id"], "1939")
            self.assertEqual(
                result["specResults"][0]["title"],
                "gwt-spec: Semantic search platform",
            )

    def test_search_specs_refreshes_existing_index_from_issue_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(
                cache_root,
                1939,
                "gwt-spec: Semantic search platform",
                "# Semantic search platform\nWatcher debounce semantics.\n",
                ["gwt-spec", "phase/review"],
            )

            db_root = Path(tmp) / "index_root"
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                initial = runner.action_search_v2(
                    action="search-specs",
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

                self.assertTrue(initial["ok"], initial)
                self.assertEqual(initial["specResults"][0]["spec_id"], "1939")

                spec_path = cache_root / "1939" / "sections" / "spec.md"
                spec_path.write_text(
                    "# Semantic search platform\nIssue cache refresh contract.\n"
                )
                refreshed = runner.action_search_v2(
                    action="search-specs",
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root=str(root),
                    query="issue cache refresh contract",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertTrue(refreshed["ok"], refreshed)
            self.assertEqual(len(refreshed["specResults"]), 1, refreshed["specResults"])
            self.assertEqual(refreshed["specResults"][0]["spec_id"], "1939")


if __name__ == "__main__":
    unittest.main()
