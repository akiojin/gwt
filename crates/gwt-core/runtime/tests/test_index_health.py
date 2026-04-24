"""Phase 6: tests for index health detection and self-heal search fallback."""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chromadb

import chroma_index_runner as runner


class IndexHealthTests(unittest.TestCase):
    REPO_HASH = "abc1234567890def"
    WORKTREE_HASH = "111122223333ffff"

    def _make_repo(self, root: Path) -> None:
        (root / "src").mkdir(parents=True)
        (root / "src" / "watcher.rs").write_text(
            "//! file system watcher with debounce\n"
            "fn debounce_events() {}\n"
        )
        (root / "README.md").write_text(
            "# Project index health\n"
            "Docs repair details live here.\n"
        )

    def _write_cached_issue(self, cache_root: Path) -> None:
        issue = cache_root / "77"
        issue.mkdir(parents=True, exist_ok=True)
        (issue / "meta.json").write_text(
            '{"number":77,"title":"Broken issue index",'
            '"labels":["bug"],"state":"open","updated_at":"2026-04-21T00:00:00Z"}'
        )
        (issue / "body.md").write_text("Search should repair missing issue collection.")

    def _write_cached_spec(self, cache_root: Path) -> None:
        issue = cache_root / "1939"
        sections = issue / "sections"
        sections.mkdir(parents=True, exist_ok=True)
        (issue / "meta.json").write_text(
            '{"number":1939,"title":"Project index SPEC",'
            '"labels":["gwt-spec","phase/review"],"state":"open",'
            '"updated_at":"2026-04-21T00:00:00Z","comment_ids":[]}'
        )
        (sections / "spec.md").write_text(
            "# Project index SPEC\nRuntime repair must rebuild corrupt stores."
        )

    def _files_db_path(self, db_root: Path) -> Path:
        return runner.resolve_db_path(
            self.REPO_HASH,
            self.WORKTREE_HASH,
            "files",
            db_root=db_root,
        )

    def _docs_db_path(self, db_root: Path) -> Path:
        return runner.resolve_db_path(
            self.REPO_HASH,
            self.WORKTREE_HASH,
            "files-docs",
            db_root=db_root,
        )

    def test_search_files_rebuilds_when_collection_is_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            result = runner.action_index_files_v2(
                project_root=str(root),
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                mode="full",
                db_root=db_root,
                scope="files",
            )
            self.assertTrue(result["ok"], result)

            db_path = self._files_db_path(db_root)
            client, collection = runner._make_chroma_collection(
                db_path,
                runner.V2_FILES_CODE_COLLECTION,
            )
            try:
                existing = collection.get()
                if existing.get("ids"):
                    collection.delete(ids=existing["ids"])
            finally:
                client.close()

            search = runner.action_search_v2(
                action="search-files",
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                project_root=str(root),
                query="watcher debounce",
                n_results=5,
                no_auto_build=False,
                db_root=db_root,
            )

            self.assertTrue(search["ok"], search)
            self.assertTrue(
                any(item["path"] == "src/watcher.rs" for item in search["results"]),
                search["results"],
            )

    def test_search_files_rebuilds_when_manifest_is_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            result = runner.action_index_files_v2(
                project_root=str(root),
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                mode="full",
                db_root=db_root,
                scope="files",
            )
            self.assertTrue(result["ok"], result)

            manifest = runner._manifest_path(self._files_db_path(db_root), "files")
            manifest.unlink()

            search = runner.action_search_v2(
                action="search-files",
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                project_root=str(root),
                query="watcher debounce",
                n_results=5,
                no_auto_build=False,
                db_root=db_root,
            )

            self.assertTrue(search["ok"], search)
            self.assertTrue(manifest.exists(), "search should repair the missing manifest")

    def test_search_files_docs_builds_missing_docs_scope(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            search = runner.action_search_v2(
                action="search-files-docs",
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                project_root=str(root),
                query="docs repair details",
                n_results=5,
                no_auto_build=False,
                db_root=db_root,
            )

            self.assertTrue(search["ok"], search)
            self.assertTrue(
                any(item["path"] == "README.md" for item in search["results"]),
                search["results"],
            )
            self.assertTrue(
                self._docs_db_path(db_root).exists(),
                "docs search should materialize the missing files-docs scope",
            )

    def test_no_auto_build_returns_index_unhealthy_for_broken_scope(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            db_path = self._files_db_path(db_root)
            client, _ = runner._make_chroma_collection(db_path, runner.V2_FILES_CODE_COLLECTION)
            client.close()

            search = runner.action_search_v2(
                action="search-files",
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                project_root=str(root),
                query="watcher",
                n_results=5,
                no_auto_build=True,
                db_root=db_root,
            )

            self.assertFalse(search["ok"])
            self.assertEqual(search.get("error_code"), "INDEX_UNHEALTHY")

    def test_search_issues_rebuilds_when_sqlite_exists_without_collection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / self.REPO_HASH
            self._write_cached_issue(cache_root)

            db_path = runner.resolve_db_path(self.REPO_HASH, None, "issues", db_root=db_root)
            db_path.mkdir(parents=True, exist_ok=True)
            client = chromadb.PersistentClient(path=str(db_path))
            client.close()

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                search = runner.action_search_v2(
                    action="search-issues",
                    repo_hash=self.REPO_HASH,
                    worktree_hash=None,
                    project_root=str(root),
                    query="missing issue collection",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertTrue(search["ok"], search)
            self.assertTrue(
                any(item["number"] == 77 for item in search["issueResults"]),
                search["issueResults"],
            )

    def test_search_issues_no_auto_build_reports_unhealthy_collection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            db_path = runner.resolve_db_path(self.REPO_HASH, None, "issues", db_root=db_root)
            db_path.mkdir(parents=True, exist_ok=True)
            client = chromadb.PersistentClient(path=str(db_path))
            client.close()

            search = runner.action_search_v2(
                action="search-issues",
                repo_hash=self.REPO_HASH,
                worktree_hash=None,
                project_root=str(root),
                query="issue",
                n_results=5,
                no_auto_build=True,
                db_root=db_root,
            )

            self.assertFalse(search["ok"])
            self.assertEqual(search.get("error_code"), "INDEX_UNHEALTHY")

    def test_index_specs_full_rebuild_retries_after_corrupt_store_open_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / self.REPO_HASH
            self._write_cached_spec(cache_root)

            original_make = runner._make_chroma_collection
            calls = {"count": 0}

            def flaky_make(db_path, collection_name):
                calls["count"] += 1
                if calls["count"] == 1:
                    raise RuntimeError("corrupt hnsw")
                return original_make(db_path, collection_name)

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                with mock.patch.object(runner, "_make_chroma_collection", flaky_make):
                    result = runner.action_index_specs_v2(
                        project_root=str(root),
                        repo_hash=self.REPO_HASH,
                        worktree_hash=None,
                        mode="full",
                        db_root=db_root,
                    )

            self.assertTrue(result["ok"], result)
            self.assertGreater(result["indexed"], 0)
            self.assertGreaterEqual(calls["count"], 2)


if __name__ == "__main__":
    unittest.main()
