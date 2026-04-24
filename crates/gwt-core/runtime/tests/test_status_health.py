"""Phase 6: tests for `status` health and repair diagnostics."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class StatusHealthTests(unittest.TestCase):
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

    def _write_cached_spec(self, cache_root: Path) -> None:
        issue = cache_root / "1939"
        sections = issue / "sections"
        sections.mkdir(parents=True, exist_ok=True)
        (issue / "meta.json").write_text(
            json.dumps(
                {
                    "number": 1939,
                    "title": "gwt-spec: Semantic search platform",
                    "labels": ["gwt-spec", "phase/review"],
                    "state": "open",
                    "updated_at": "2026-04-21T00:00:00Z",
                    "comment_ids": [],
                }
            )
        )
        (sections / "spec.md").write_text(
            "# Semantic search platform\n"
            "Health diagnostics and repair metadata.\n"
        )

    def _worktree_root(self, db_root: Path) -> Path:
        return (
            db_root
            / self.REPO_HASH
            / "worktrees"
            / self.WORKTREE_HASH
        )

    def test_status_reports_repair_required_for_missing_manifest_and_docs_scope(self):
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

            manifest = runner._manifest_path(self._worktree_root(db_root), "files")
            manifest.unlink()

            status = runner.action_status_v2(
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                db_root=db_root,
            )
            self.assertTrue(status["ok"], status)
            self.assertEqual(status["runtime"]["reason"], "ready")
            self.assertTrue(status["runtime"]["healthy"])
            self.assertFalse(status["runtime"]["repaired"])
            self.assertRegex(status["runtime"]["asset_hash"], r"^[0-9a-f]{16}$")
            self.assertEqual(status["runtime"]["smoke_test"], "passed")

            files = status["status"]["files"]
            docs = status["status"]["files-docs"]

            self.assertTrue(files["exists"])
            self.assertFalse(files["healthy"])
            self.assertTrue(files["repair_required"])
            self.assertEqual(files["reason"], "manifest_missing")
            self.assertEqual(files["document_count"], 1)
            self.assertIsNotNone(files["last_repair_at"])

            self.assertFalse(docs["exists"])
            self.assertFalse(docs["healthy"])
            self.assertTrue(docs["repair_required"])
            self.assertEqual(docs["reason"], "collection_missing")
            self.assertEqual(docs["document_count"], 0)

    def test_status_reports_legacy_residue_and_last_repair_at(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)
            db_root = Path(tmp) / "index_root"

            for scope in ("files", "files-docs"):
                result = runner.action_index_files_v2(
                    project_root=str(root),
                    repo_hash=self.REPO_HASH,
                    worktree_hash=self.WORKTREE_HASH,
                    mode="full",
                    db_root=db_root,
                    scope=scope,
                )
                self.assertTrue(result["ok"], result)

            worktree_root = self._worktree_root(db_root)
            legacy_specs = worktree_root / "specs"
            legacy_specs.mkdir(parents=True, exist_ok=True)
            (legacy_specs / "chroma.sqlite3").write_text("legacy")
            (worktree_root / "manifest-specs.json").write_text("[]")

            status = runner.action_status_v2(
                repo_hash=self.REPO_HASH,
                worktree_hash=self.WORKTREE_HASH,
                db_root=db_root,
            )
            self.assertTrue(status["ok"], status)

            files = status["status"]["files"]
            self.assertFalse(files["healthy"])
            self.assertTrue(files["repair_required"])
            self.assertEqual(files["reason"], "legacy_residue")
            self.assertTrue(files["legacy_residue_detected"])
            self.assertIsNotNone(files["last_repair_at"])

    def test_status_reports_repo_scoped_specs_without_worktree_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / self.REPO_HASH
            self._write_cached_spec(cache_root)

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_index_specs_v2(
                    project_root=str(root),
                    repo_hash=self.REPO_HASH,
                    worktree_hash=None,
                    mode="full",
                    db_root=db_root,
                )
            self.assertTrue(result["ok"], result)

            status = runner.action_status_v2(
                repo_hash=self.REPO_HASH,
                worktree_hash=None,
                db_root=db_root,
            )
            self.assertTrue(status["ok"], status)

            specs = status["status"]["specs"]
            self.assertTrue(specs["exists"])
            self.assertTrue(specs["healthy"])
            self.assertFalse(specs["repair_required"])
            self.assertGreaterEqual(specs["document_count"], 1)
            self.assertEqual(specs["reason"], "ready")
            self.assertFalse(specs["legacy_residue_detected"])
            self.assertIsNotNone(specs["last_repair_at"])

    def test_status_allows_chunked_specs_to_have_more_records_than_manifest_entries(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            db_path = runner.resolve_db_path(
                self.REPO_HASH,
                None,
                "specs",
                db_root=db_root,
            )
            db_path.mkdir(parents=True)
            (db_path / "chroma.sqlite3").write_text("placeholder")
            runner.write_manifest(
                db_path,
                scope="specs",
                entries=[{"path": "1939", "mtime": 1, "size": 2}],
            )

            with mock.patch.object(
                runner, "_scope_document_count", return_value=(True, 5)
            ):
                specs = runner._scope_status_v2(
                    self.REPO_HASH,
                    None,
                    "specs",
                    db_root=db_root,
                )

            self.assertTrue(specs["healthy"], specs)
            self.assertFalse(specs["repair_required"], specs)
            self.assertEqual(specs["reason"], "ready")
            self.assertEqual(specs["document_count"], 5)


if __name__ == "__main__":
    unittest.main()
