"""Phase 8: tests for manifest.json based incremental indexing.

Files index actions must persist (path, mtime, size) tuples and only re-embed
the diff on subsequent runs.
"""

from __future__ import annotations

import json
import os
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


def _make_repo(root: Path) -> None:
    (root / "src").mkdir(parents=True)
    (root / "src" / "a.rs").write_text("// a\nfn a() {}\n")
    (root / "src" / "b.rs").write_text("// b\nfn b() {}\n")
    (root / "src" / "c.rs").write_text("// c\nfn c() {}\n")
    (root / "src" / "d.rs").write_text("// d\nfn d() {}\n")
    (root / "src" / "e.rs").write_text("// e\nfn e() {}\n")


class ManifestHelperTests(unittest.TestCase):
    def test_helpers_exist(self):
        self.assertTrue(hasattr(runner, "read_manifest"))
        self.assertTrue(hasattr(runner, "write_manifest"))
        self.assertTrue(hasattr(runner, "compute_manifest_diff"))

    def test_write_then_read_round_trip(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp)
            entries = [
                {"path": "src/a.rs", "mtime": 1700000000, "size": 100},
                {"path": "src/b.rs", "mtime": 1700000001, "size": 200},
            ]
            runner.write_manifest(db, scope="files", entries=entries)
            loaded = runner.read_manifest(db, scope="files")
            self.assertEqual(loaded, entries)

    def test_read_manifest_returns_empty_when_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp)
            loaded = runner.read_manifest(db, scope="files")
            self.assertEqual(loaded, [])

    def test_compute_manifest_diff_detects_added(self):
        old = [{"path": "a", "mtime": 1, "size": 1}]
        new = [
            {"path": "a", "mtime": 1, "size": 1},
            {"path": "b", "mtime": 2, "size": 2},
        ]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["added"], ["b"])
        self.assertEqual(diff["changed"], [])
        self.assertEqual(diff["removed"], [])

    def test_compute_manifest_diff_detects_changed_by_mtime(self):
        old = [{"path": "a", "mtime": 1, "size": 100}]
        new = [{"path": "a", "mtime": 2, "size": 100}]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["changed"], ["a"])

    def test_compute_manifest_diff_detects_changed_by_size(self):
        old = [{"path": "a", "mtime": 1, "size": 100}]
        new = [{"path": "a", "mtime": 1, "size": 200}]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["changed"], ["a"])

    def test_compute_manifest_diff_detects_removed(self):
        old = [
            {"path": "a", "mtime": 1, "size": 1},
            {"path": "b", "mtime": 1, "size": 1},
        ]
        new = [{"path": "a", "mtime": 1, "size": 1}]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["removed"], ["b"])


class IncrementalIndexingTests(unittest.TestCase):
    def test_full_index_writes_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _make_repo(root)

            result = runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="full",
                db_root=Path(tmp) / "index_root",
            )
            self.assertTrue(result["ok"], result)

            db = (
                Path(tmp)
                / "index_root"
                / "abc1234567890def"
                / "worktrees"
                / "111122223333ffff"
            )
            manifest = runner.read_manifest(db, scope="files")
            self.assertGreaterEqual(len(manifest), 5)
            for entry in manifest:
                self.assertIn("path", entry)
                self.assertIn("mtime", entry)
                self.assertIn("size", entry)

    def test_incremental_only_reembeds_changed_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _make_repo(root)
            db_root = Path(tmp) / "index_root"

            runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="full",
                db_root=db_root,
            )

            # Modify exactly one file. Sleep to ensure mtime changes.
            time.sleep(1.05)
            (root / "src" / "b.rs").write_text("// b modified\nfn b() {}\n")

            with mock.patch.object(
                runner, "embed_documents_for_paths", wraps=runner.embed_documents_for_paths
            ) as spy:
                result = runner.action_index_files_v2(
                    project_root=str(root),
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    mode="incremental",
                    db_root=db_root,
                )
            self.assertTrue(result["ok"])
            # The spy must have been called with exactly one path.
            paths_passed = []
            for call in spy.call_args_list:
                paths_passed.extend(call[0][0])
            self.assertEqual(
                len(paths_passed),
                1,
                f"incremental should re-embed only the changed file, got {paths_passed}",
            )
            self.assertTrue(str(paths_passed[0]).endswith("b.rs"))

    def test_incremental_detects_deleted_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _make_repo(root)
            db_root = Path(tmp) / "index_root"

            runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="full",
                db_root=db_root,
            )

            (root / "src" / "c.rs").unlink()

            result = runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="incremental",
                db_root=db_root,
            )
            self.assertTrue(result["ok"])

            db = (
                db_root
                / "abc1234567890def"
                / "worktrees"
                / "111122223333ffff"
            )
            manifest = runner.read_manifest(db, scope="files")
            paths = {e["path"] for e in manifest}
            self.assertNotIn("src/c.rs", paths)


if __name__ == "__main__":
    unittest.main()
