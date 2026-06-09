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

    def test_search_multi_disables_auto_build_for_each_scope(self):
        with mock.patch.object(
            runner,
            "action_search_v2",
            side_effect=[
                {"ok": True, "issueResults": [{"number": 1}]},
                {"ok": True, "specResults": [{"spec_id": "1939"}]},
                {"ok": True, "boardResults": [{"entry_id": "board-1"}]},
            ],
        ) as search:
            result = runner.action_search_multi_v2(
                repo_hash="abc1234567890def",
                worktree_hash=None,
                project_root="/repo",
                query="Git",
                n_results=5,
                scopes=["issues", "specs", "board"],
            )

        self.assertTrue(result["ok"], result)
        self.assertEqual(result["issueResults"][0]["number"], 1)
        self.assertEqual(result["specResults"][0]["spec_id"], "1939")
        self.assertEqual(result["boardResults"][0]["entry_id"], "board-1")
        self.assertEqual(search.call_count, 3)
        for call in search.call_args_list:
            self.assertTrue(call.kwargs["no_auto_build"])

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
                    worktree_hash=None,
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
            db = db_root / "abc1234567890def" / "specs"
            self.assertTrue(db.exists(), f"index dir was not created: {db}")

    def test_search_memory_auto_builds_when_index_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            (root / ".gwt" / "work").mkdir(parents=True)
            (root / ".gwt" / "work" / "memory.md").write_text(
                "# Memory Learned\n\n"
                "## 2026-05-20 — watcher debounce regression\n\n"
                "### 事象\n watcher fired too often.\n\n"
                "### 原因\n debounce too low.\n\n"
                "### 再発防止策\n raise debounce.\n",
                encoding="utf-8",
            )

            db_root = Path(tmp) / "index_root"
            result = runner.action_search_v2(
                action="search-memory",
                repo_hash="abc1234567890def",
                worktree_hash=None,
                project_root=str(root),
                query="watcher debounce",
                n_results=5,
                no_auto_build=False,
                db_root=db_root,
            )

            self.assertTrue(result["ok"], result)
            self.assertIn("memoryResults", result)
            self.assertGreaterEqual(len(result["memoryResults"]), 1, result["memoryResults"])
            top = result["memoryResults"][0]
            self.assertEqual(top["date"], "2026-05-20")
            self.assertIn("watcher debounce", top["title"])
            db = db_root / "abc1234567890def" / "memory"
            self.assertTrue(db.exists(), f"memory index dir was not created: {db}")

    def test_search_memory_returns_index_missing_when_no_auto_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            (root / ".gwt" / "work").mkdir(parents=True)
            (root / ".gwt" / "work" / "memory.md").write_text("# empty\n", encoding="utf-8")
            db_root = Path(tmp) / "index_root"

            result = runner.action_search_v2(
                action="search-memory",
                repo_hash="abc1234567890def",
                worktree_hash=None,
                project_root=str(root),
                query="anything",
                n_results=5,
                no_auto_build=True,
                db_root=db_root,
            )

            self.assertFalse(result["ok"], result)
            self.assertEqual(result["error_code"], "INDEX_MISSING")

    def test_search_specs_empty_corpus_returns_diagnostic_when_cache_unpopulated(self):
        # Issue #2979: when the issue cache is empty/unpopulated, an auto-built
        # spec index has zero documents. The runner must NOT silently succeed
        # with `ok: true, specResults: []` (which agents misread as "no SPEC
        # owner exists"); it must return a non-OK diagnostic.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_search_v2(
                    action="search-specs",
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertFalse(result["ok"], result)
            self.assertEqual(result.get("error_code"), "EMPTY_CORPUS", result)
            self.assertEqual(result.get("scope"), "specs", result)
            self.assertIn("cache", result.get("error", "").lower(), result)

    def test_search_issues_empty_corpus_returns_diagnostic_when_cache_unpopulated(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_search_v2(
                    action="search-issues",
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    project_root=str(root),
                    query="anything",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertFalse(result["ok"], result)
            self.assertEqual(result.get("error_code"), "EMPTY_CORPUS", result)
            self.assertEqual(result.get("scope"), "issues", result)

    def test_search_specs_returns_empty_results_when_cache_has_no_specs(self):
        # A populated issue cache that simply contains no `gwt-spec` issues is a
        # legitimate empty result, not a tooling failure. The runner must keep
        # returning `ok: true, specResults: []` here (no false positive).
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(
                cache_root,
                2000,
                "Plain issue",
                "# Plain issue\nNo spec label here.\n",
                ["bug"],
            )
            db_root = Path(tmp) / "index_root"
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_search_v2(
                    action="search-specs",
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertTrue(result["ok"], result)
            self.assertEqual(result.get("specResults"), [], result)

    def test_no_auto_build_empty_index_does_not_emit_empty_corpus_diagnostic(self):
        # The interactive GUI search path (search-multi) always passes
        # no_auto_build=True and must not fail the whole multi-scope search just
        # because one scope's existing index is empty. The diagnostic is scoped
        # to the agent auto-build preflight only.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                build = runner.action_index_specs_v2(
                    project_root=str(root),
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    mode="full",
                    db_root=db_root,
                )
                self.assertTrue(build["ok"], build)
                self.assertEqual(build["indexed"], 0, build)

                result = runner.action_search_v2(
                    action="search-specs",
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    no_auto_build=True,
                    db_root=db_root,
                )

            self.assertTrue(result["ok"], result)
            self.assertEqual(result.get("specResults"), [], result)

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
                    worktree_hash=None,
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
                    worktree_hash=None,
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
