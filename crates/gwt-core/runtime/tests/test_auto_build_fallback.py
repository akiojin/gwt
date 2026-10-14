"""Phase 8: tests for the search-* auto-build fallback.

When a search action is invoked against a missing index, the runner must
build the index in-process (full mode) and then run the search. The
--no-auto-build flag suppresses this behavior.
"""

from __future__ import annotations

import argparse
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

    @staticmethod
    def _file_results(payload: dict) -> list[dict]:
        return list(
            ((payload.get("scope_results") or {}).get("files") or {}).get(
                "results"
            )
            or []
        )

    def test_explicit_v2_search_serves_healthy_legacy_without_eager_migration(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"
            coordinator = Path(tmp) / "coordinator"
            coordinator.mkdir()
            repo_hash = "abc1234567890def"
            worktree_hash = "111122223333ffff"
            with mock.patch.dict(
                os.environ,
                {
                    "GWT_INDEX_COORDINATOR_ROOT": str(coordinator),
                    "GWT_INDEX_FAKE_EMBEDDING": "1",
                },
                clear=False,
            ):
                for scope in ("files", "files-docs"):
                    built = runner.action_index_files_v2(
                        project_root=str(root),
                        repo_hash=repo_hash,
                        worktree_hash=worktree_hash,
                        mode="full",
                        db_root=db_root,
                        scope=scope,
                        file_index_protocol="legacy",
                    )
                    self.assertTrue(built.get("ok"), built)
                legacy_db = runner.resolve_db_path(
                    repo_hash, worktree_hash, "files", db_root=db_root
                )
                legacy_pointer = runner.active_pointer_path(legacy_db)
                pointer_before = legacy_pointer.read_bytes()
                v2_root = runner.resolve_file_index_v2_root(
                    repo_hash, db_root=db_root
                )
                self.assertFalse(v2_root.exists())
                sentinel = v2_root / "sentinel.bin"
                sentinel.parent.mkdir(parents=True)
                sentinel.write_bytes(b"v2-layout-must-not-change")

                with mock.patch.object(runner, "action_index_files_v2") as rebuild:
                    legacy = runner.action_search_v2(
                        action="search-files",
                        repo_hash=repo_hash,
                        worktree_hash=worktree_hash,
                        project_root=str(root),
                        query="watcher debounce",
                        n_results=5,
                        db_root=db_root,
                    )
                    legacy_docs = runner.action_search_v2(
                        action="search-files-docs",
                        repo_hash=repo_hash,
                        worktree_hash=worktree_hash,
                        project_root=str(root),
                        query="project",
                        n_results=5,
                        db_root=db_root,
                    )
                    explicit_single = runner.action_search_v2(
                        action="search-files",
                        repo_hash=repo_hash,
                        worktree_hash=worktree_hash,
                        project_root=str(root),
                        query="watcher debounce",
                        n_results=5,
                        db_root=db_root,
                        file_index_protocol="v2",
                    )
                    explicit_v2 = runner.action_search_multi_v2(
                        repo_hash=repo_hash,
                        worktree_hash=worktree_hash,
                        project_root=str(root),
                        query="watcher debounce",
                        n_results=5,
                        scopes=["files"],
                        db_root=db_root,
                        file_index_protocol="v2",
                    )
                    runner._MODEL_CACHE = None
                    cache_cleared = runner.action_search_multi_v2(
                        repo_hash=repo_hash,
                        worktree_hash=worktree_hash,
                        project_root=str(root),
                        query="watcher debounce",
                        n_results=5,
                        scopes=["files"],
                        db_root=db_root,
                        file_index_protocol="v2",
                    )

                rebuild.assert_not_called()
                self.assertTrue(legacy.get("ok"), legacy)
                self.assertTrue(legacy.get("results"), legacy)
                self.assertTrue(legacy_docs.get("ok"), legacy_docs)
                self.assertTrue(legacy_docs.get("results"), legacy_docs)
                self.assertTrue(explicit_single.get("ok"), explicit_single)
                self.assertTrue(explicit_single.get("results"), explicit_single)
                self.assertEqual(
                    explicit_single.get("fallback_source"), "legacy", explicit_single
                )
                for result in (explicit_v2, cache_cleared):
                    self.assertTrue(result.get("ok"), result)
                    self.assertTrue(self._file_results(result), result)
                    self.assertEqual(
                        result["scopes"]["files"].get("fallback_source"),
                        "legacy",
                        result,
                    )
                    self.assertEqual(
                        result["scopes"]["files"].get("state"), "stale", result
                    )
                    self.assertEqual(result.get("stale_scopes"), ["files"], result)
                self.assertEqual(legacy_pointer.read_bytes(), pointer_before)
                self.assertEqual(
                    sorted(path.relative_to(v2_root) for path in v2_root.rglob("*")),
                    [Path("sentinel.bin")],
                    "read-only legacy/default/v2 fallback must not mutate v2 layout",
                )
                self.assertEqual(sentinel.read_bytes(), b"v2-layout-must-not-change")

    def test_explicit_v2_search_without_any_compatible_corpus_is_typed_not_ready(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"
            with mock.patch.object(runner, "action_index_files_v2") as rebuild:
                single = runner.action_search_v2(
                    action="search-files",
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    db_root=db_root,
                    file_index_protocol="v2",
                )
                result = runner.action_search_multi_v2(
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root=str(root),
                    query="watcher debounce",
                    n_results=5,
                    scopes=["files"],
                    db_root=db_root,
                    file_index_protocol="v2",
                )

            rebuild.assert_not_called()
            self.assertFalse(single.get("ok"), single)
            self.assertEqual(single.get("error_code"), "INDEX_NOT_READY", single)
            self.assertIs(single.get("retryable"), True, single)
            self.assertEqual(single.get("waited_ms"), 0, single)
            self.assertNotIn("results", single)
            self.assertFalse(result.get("ok"), result)
            self.assertEqual(result.get("error_code"), "INDEX_NOT_READY", result)
            self.assertEqual(result.get("waited_ms"), 0, result)
            self.assertIs(result.get("retryable"), True, result)
            self.assertEqual(result.get("affected_scopes"), ["files"], result)
            self.assertGreater(result.get("retry_after_ms", 0), 0, result)
            self.assertNotIn("scope_results", result)

    def test_single_file_search_dispatch_forwards_explicit_protocol(self):
        for action in ("search-files", "search-files-docs"):
            with self.subTest(action=action):
                args = argparse.Namespace(
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    db_root="",
                    project_root="/fixture/project",
                    query="watcher debounce",
                    n_results=3,
                    no_auto_build=False,
                    match_mode="semantic",
                    file_index_protocol="v2",
                )
                expected = {"ok": False, "error_code": "INDEX_NOT_READY"}
                with mock.patch.object(
                    runner, "action_search_v2", return_value=expected
                ) as search, mock.patch.object(runner, "emit") as emit:
                    exit_code = runner._dispatch_v2(action, args)

                self.assertEqual(exit_code, 0)
                search.assert_called_once_with(
                    action=action,
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    project_root="/fixture/project",
                    query="watcher debounce",
                    n_results=3,
                    no_auto_build=False,
                    match_mode="semantic",
                    db_root=None,
                    file_index_protocol="v2",
                )
                emit.assert_called_once_with(expected)

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

    def test_search_multi_never_auto_builds_missing_scopes(self):
        # Phase 70: search-multi classifies broken scopes instead of building
        # them inline. Missing stores are reported per scope; no index action
        # runs (the Rust caller owns repair scheduling).
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index"
            with mock.patch.object(
                runner, "action_index_issues_v2"
            ) as build_issues, mock.patch.object(
                runner, "action_index_specs_v2"
            ) as build_specs:
                result = runner.action_search_multi_v2(
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    project_root=str(Path(tmp) / "repo"),
                    query="Git",
                    n_results=5,
                    scopes=["issues", "specs", "board"],
                    db_root=db_root,
                )

        self.assertTrue(result["ok"], result)
        for scope in ("issues", "specs", "board"):
            self.assertEqual(
                result["scopes"][scope]["state"], "missing", result
            )
        build_issues.assert_not_called()
        build_specs.assert_not_called()

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
