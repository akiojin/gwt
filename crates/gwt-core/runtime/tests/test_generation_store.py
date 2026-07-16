"""Phase 70 T-IDX-391 (Issue #3264): atomic generation store fault contract.

FR-390: full rebuilds construct an immutable staging generation and publish
it by atomically replacing the `active.json` pointer. The live store is
never reset in place: kill / crash / disk full at any boundary leaves the
previous healthy generation searchable, and an incomplete generation is
never exposed to readers. Legacy stores stay readable and lazily migrate on
the next update (AS-17); abandoned generations older than 24 hours are the
only GC target.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner

RUNNER_PATH = Path(runner.__file__).resolve()

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


def _make_project(base: Path, docs: int) -> Path:
    project = base / "project"
    src = project / "src"
    src.mkdir(parents=True, exist_ok=True)
    for index in range(docs):
        (src / f"module_{index:02}.rs").write_text(
            f"//! module {index}\nfn feature_{index}() {{}}\n", encoding="utf-8"
        )
    return project


class GenerationStoreTests(unittest.TestCase):
    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        self.coord = self.base / "coordinator"
        self.coord.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(self.coord),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def _build(self, project: Path) -> dict:
        return runner.action_index_files_v2(
            project_root=str(project),
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            mode="full",
            db_root=self.db_root,
            scope="files",
        )

    def _db_path(self) -> Path:
        return runner.resolve_db_path(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )

    def _status(self) -> dict:
        return runner._scope_status_v2(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )

    def _search(self, query: str) -> dict:
        return runner._search_scope_collection(
            REPO_HASH,
            WORKTREE_HASH,
            "files",
            query,
            5,
            "semantic",
            self.db_root,
            None,
        )

    def test_publish_creates_generation_with_atomic_active_pointer(self):
        project = _make_project(self.base, 6)
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)

        db_path = self._db_path()
        pointer = runner.active_pointer_path(db_path)
        self.assertTrue(
            pointer.is_file(),
            f"publish must write the active.json pointer, missing at {pointer}",
        )
        active = json.loads(pointer.read_text(encoding="utf-8"))
        generation_dir = pointer.parent / active["generation"]
        self.assertTrue(
            (generation_dir / "chroma.sqlite3").exists(),
            f"active pointer must reference a complete generation: {active}",
        )
        resolved = runner.resolve_active_store(db_path)
        self.assertEqual(
            resolved,
            generation_dir,
            "readers must resolve through the active pointer",
        )
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 6, status)
        self.assertTrue(self._search("module feature").get("results"), "search works")

    def test_kill_during_staging_build_preserves_active_generation(self):
        project = _make_project(self.base, 48)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        # Change every document so the rebuild has real embedding work
        # (unchanged records would be reused without checkpoints, FR-391).
        for index in range(48):
            (project / "src" / f"module_{index:02}.rs").write_text(
                f"//! module {index} v2\nfn feature_{index}_v2() {{}}\n",
                encoding="utf-8",
            )

        home = self.base / "home"
        home.mkdir(exist_ok=True)
        env = os.environ.copy()
        env["HOME"] = str(home)
        env["USERPROFILE"] = str(home)
        env["GWT_INDEX_FAKE_EMBEDDING"] = "1"
        env["GWT_INDEX_COORDINATOR_ROOT"] = str(self.coord)
        proc = subprocess.Popen(
            [
                sys.executable,
                str(RUNNER_PATH),
                "--action",
                "index-files",
                "--repo-hash",
                REPO_HASH,
                "--worktree-hash",
                WORKTREE_HASH,
                "--project-root",
                str(project),
                "--mode",
                "full",
                "--scope",
                "files",
                "--qos",
                "background",
                "--db-root",
                str(self.db_root),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        # Kill the rebuild after its first checkpoint (mid-staging-build).
        killed = False
        deadline = time.monotonic() + 60
        assert proc.stderr is not None
        while time.monotonic() < deadline:
            line = proc.stderr.readline()
            if not line:
                break
            if '"done": 16' in line or '"done":16' in line:
                proc.kill()
                killed = True
                break
        proc.wait(timeout=30)
        self.assertTrue(killed, "expected to kill the rebuild at a checkpoint")

        # AS-9: the previous healthy generation stays active and searchable.
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 48, status)
        self.assertTrue(self._search("module feature").get("results"))

        # A follow-up rebuild completes normally.
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("indexed"), 48, result)

    def test_disk_full_during_publish_keeps_previous_generation(self):
        project = _make_project(self.base, 6)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        (project / "src" / "module_00.rs").write_text(
            "//! module 0 changed\nfn feature_0_changed() {}\n", encoding="utf-8"
        )
        real_replace = os.replace

        def failing_replace(src, dst, *args, **kwargs):
            if str(dst).endswith("active.json"):
                raise OSError(28, "No space left on device")
            return real_replace(src, dst, *args, **kwargs)

        with mock.patch("os.replace", side_effect=failing_replace):
            result = self._build(project)

        self.assertFalse(
            result.get("ok"),
            f"a failed publish must not report silent success: {result}",
        )
        self.assertEqual(result.get("error_code"), "PUBLISH_FAILED", result)
        # AS-9: previous generation still active and searchable.
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 6, status)

    def test_corrupt_active_pointer_is_classified_for_repair_not_crash(self):
        project = _make_project(self.base, 4)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        pointer = runner.active_pointer_path(self._db_path())
        pointer.write_text("{not valid json", encoding="utf-8")

        status = self._status()
        self.assertTrue(
            status["repair_required"],
            f"corrupt active pointer must classify as repair-required: {status}",
        )

        # Rebuild repairs the pointer.
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 4, status)

    def test_legacy_layout_serves_reads_and_migrates_on_next_update(self):
        # AS-17: a pre-generation store (chroma directly under the scope dir)
        # keeps serving reads without a rebuild and migrates on next update.
        project = _make_project(self.base, 3)
        db_path = self._db_path()
        db_path.mkdir(parents=True, exist_ok=True)
        client, collection = runner._make_chroma_collection(
            db_path, runner.V2_FILES_CODE_COLLECTION
        )
        try:
            paths = sorted((project / "src").glob("*.rs"))
            runner.embed_documents_for_paths(paths, project, collection)
        finally:
            runner._close_chroma_client(client)
        entries = runner._build_manifest_entries(
            project, sorted((project / "src").glob("*.rs"))
        )
        runner.write_manifest(db_path, scope="files", entries=entries)
        runner._write_scope_meta(
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            scope="files",
            db_root=self.db_root,
            updates={"document_count": 3},
        )

        status = self._status()
        self.assertTrue(status["healthy"], f"legacy store must stay readable: {status}")
        self.assertEqual(status["document_count"], 3, status)
        self.assertTrue(self._search("module feature").get("results"))

        # Next update publishes a generation without a startup mass rebuild.
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)
        self.assertTrue(runner.active_pointer_path(db_path).is_file())
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 3, status)

    def test_abandoned_generations_are_gc_only_after_24_hours(self):
        project = _make_project(self.base, 3)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        pointer = runner.active_pointer_path(self._db_path())
        generations_root = pointer.parent
        old_abandoned = generations_root / "gen-old-abandoned"
        old_abandoned.mkdir()
        (old_abandoned / "chroma.sqlite3").write_text("old", encoding="utf-8")
        stale_time = time.time() - 25 * 3600
        os.utime(old_abandoned, (stale_time, stale_time))
        fresh_abandoned = generations_root / "gen-fresh-abandoned"
        fresh_abandoned.mkdir()
        (fresh_abandoned / "chroma.sqlite3").write_text("fresh", encoding="utf-8")

        result = self._build(project)
        self.assertTrue(result.get("ok"), result)

        self.assertFalse(
            old_abandoned.exists(),
            "generations abandoned for more than 24h must be garbage collected",
        )
        self.assertTrue(
            fresh_abandoned.exists(),
            "recently abandoned generations must be retained (crash recovery)",
        )
        active = json.loads(pointer.read_text(encoding="utf-8"))
        self.assertTrue((generations_root / active["generation"]).exists())


if __name__ == "__main__":
    unittest.main()
