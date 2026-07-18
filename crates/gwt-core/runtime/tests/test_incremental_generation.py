"""Phase 70 T-IDX-392 (Issue #3264): incremental generation build contract.

FR-391: additions / changes / deletions are detected through stable record
IDs and content hashes; unchanged records reuse their existing embeddings
from the previous healthy generation, only changed records are encoded, and
deleted IDs never reach the new generation. FR-392 / AS-10: when the source
changes between staging and publish, the late revalidation aborts the
publish and the active pointer stays on the previous generation.
"""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


class _IncrementalFixture(unittest.TestCase):
    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        coord = self.base / "coordinator"
        coord.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(coord),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()
        self.project = self.base / "project"
        self.src = self.project / "src"
        self.src.mkdir(parents=True)
        for index in range(8):
            self._write_doc(index, f"//! module {index}\nfn feature_{index}() {{}}\n")

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def _write_doc(self, index: int, body: str) -> Path:
        path = self.src / f"module_{index:02}.rs"
        path.write_text(body, encoding="utf-8")
        return path

    def _build(self) -> dict:
        return runner.action_index_files_v2(
            project_root=str(self.project),
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

    def _store_ids(self) -> set:
        store = runner.resolve_active_store(self._db_path())
        client, collection = runner._open_chroma_collection(
            store, runner.V2_FILES_CODE_COLLECTION
        )
        try:
            return set(collection.get().get("ids") or [])
        finally:
            runner._close_chroma_client(client)


class IncrementalGenerationTests(_IncrementalFixture):
    def test_rebuild_reuses_unchanged_embeddings_and_encodes_only_changes(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("indexed"), 8, baseline)

        # 2 changed, 1 added, 1 deleted.
        self._write_doc(0, "//! module 0 rewritten\nfn feature_0_v2() {}\n")
        self._write_doc(1, "//! module 1 rewritten\nfn feature_1_v2() {}\n")
        added = self._write_doc(90, "//! module 90 added\nfn feature_90() {}\n")
        removed = self.src / "module_07.rs"
        removed.unlink()

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_batches = []

        def record_encode(texts):
            encoded_batches.append(list(texts))
            return real_model.encode(texts)

        counting.encode.side_effect = record_encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            result = self._build()

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(
            result.get("indexed"), 8, f"8 - 1 deleted + 1 added = 8: {result}"
        )
        self.assertEqual(
            result.get("newly_embedded"),
            3,
            f"only the 2 changed + 1 added records may be encoded: {result}",
        )
        self.assertEqual(
            result.get("reused_embeddings"),
            5,
            f"unchanged records must reuse previous embeddings: {result}",
        )
        encoded_count = sum(len(batch) for batch in encoded_batches)
        self.assertEqual(
            encoded_count,
            3,
            f"model must only encode changed documents, encoded: {encoded_batches}",
        )

        ids = self._store_ids()
        self.assertIn("src/module_90.rs", ids, ids)
        self.assertNotIn(
            "src/module_07.rs",
            ids,
            "deleted records must be excluded from the new generation",
        )

    def test_unchanged_source_rebuild_encodes_nothing(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        counting.encode.side_effect = real_model.encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            result = self._build()

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("newly_embedded"), 0, result)
        self.assertEqual(result.get("reused_embeddings"), 8, result)
        self.assertEqual(
            counting.encode.call_count,
            0,
            "an unchanged corpus must not re-encode anything (FR-391)",
        )

    def test_source_change_during_build_aborts_publish_and_keeps_active(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)

        # Change one record so the rebuild embeds something, and mutate a
        # DIFFERENT source file during the embedding phase: the late
        # revalidation (FR-392 / AS-10) must detect the drift after taking
        # the publish boundary and refuse to publish the stale generation.
        self._write_doc(2, "//! module 2 rewritten\nfn feature_2_v2() {}\n")
        real_model = runner._FakeEmbeddingModel()
        mutating = mock.MagicMock()
        state = {"mutated": False}

        def mutate_once(texts):
            if not state["mutated"]:
                state["mutated"] = True
                self._write_doc(
                    5, "//! module 5 mutated mid-build\nfn feature_5_v3() {}\n"
                )
            return real_model.encode(texts)

        mutating.encode.side_effect = mutate_once
        with mock.patch.object(runner, "_get_embedding_model", return_value=mutating):
            result = self._build()

        self.assertFalse(
            result.get("ok"),
            f"a stale generation must not be published silently: {result}",
        )
        self.assertEqual(result.get("error_code"), "SOURCE_CHANGED", result)
        self.assertTrue(result.get("retryable"), result)

        # The active pointer still serves the previous healthy generation.
        status = runner._scope_status_v2(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )
        self.assertEqual(status["document_count"], 8, status)

class IncrementalModeRoutingTests(_IncrementalFixture):
    """PR #3301 review: `mode=\"incremental\"` must update the store that
    readers actually resolve (the active generation), not the legacy
    `db_path` that full mode has already migrated away from."""

    def _build_incremental(self) -> dict:
        return runner.action_index_files_v2(
            project_root=str(self.project),
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            mode="incremental",
            db_root=self.db_root,
            scope="files",
        )

    def test_incremental_mode_updates_the_active_generation(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        db_path = self._db_path()
        self.assertTrue(
            runner.active_pointer_path(db_path).is_file(),
            "full mode must publish an active generation first",
        )

        self._write_doc(90, "//! module 90 added\nfn feature_90() {}\n")
        (self.src / "module_07.rs").unlink()
        result = self._build_incremental()
        self.assertTrue(result.get("ok"), result)

        ids = self._store_ids()
        self.assertIn(
            "src/module_90.rs",
            ids,
            "incremental additions must reach the active generation readers use",
        )
        self.assertNotIn(
            "src/module_07.rs",
            ids,
            "incremental deletions must reach the active generation readers use",
        )
        self.assertFalse(
            (db_path / "chroma.sqlite3").exists(),
            "incremental mode must not resurrect the migrated legacy store",
        )


class PublishVerificationTests(unittest.TestCase):
    """PR #3301 review (Critical): a staging build whose verification read
    fails must never replace the healthy active generation."""

    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        coord = self.base / "coordinator"
        coord.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(coord),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def test_unverifiable_staging_is_not_published(self):
        project = self.base / "project"
        project.mkdir()
        baseline = runner.action_index_board_v2(
            repo_hash=REPO_HASH,
            project_root=str(project),
            mode="full",
            db_root=self.db_root,
        )
        self.assertTrue(baseline.get("ok"), baseline)
        db_path = runner.resolve_db_path(REPO_HASH, None, "board", db_root=self.db_root)
        pointer_before = runner._read_active_pointer(db_path)
        self.assertIsNotNone(pointer_before)

        with mock.patch.object(
            runner, "_open_chroma_collection", side_effect=RuntimeError("cannot open")
        ):
            result = runner.action_index_board_v2(
                repo_hash=REPO_HASH,
                project_root=str(project),
                mode="full",
                db_root=self.db_root,
            )

        self.assertFalse(
            result.get("ok"),
            f"an unverifiable staging build must not publish silently: {result}",
        )
        self.assertEqual(result.get("error_code"), "BUILD_VERIFY_FAILED", result)
        pointer_after = runner._read_active_pointer(db_path)
        self.assertEqual(
            pointer_before,
            pointer_after,
            "the healthy active generation must remain in place",
        )



if __name__ == "__main__":
    unittest.main()
