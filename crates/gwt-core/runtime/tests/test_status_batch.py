"""Phase 70 T-IDX-394/395 (Issue #3264): batch all-worktree status contract.

FR-393 / AS-13: explicit all-worktree health is served by one batch runner
process; the payload reports every requested worktree with the same
per-scope shape as the single-worktree status.
"""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner

REPO_HASH = "abc1234567890def"
WT_BUILT = "111122223333ffff"
WT_MISSING = "aaaa2222bbbb4444"


class StatusBatchTests(unittest.TestCase):
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

    def test_status_reports_all_requested_worktrees_in_one_payload(self):
        project = self.base / "project"
        (project / "src").mkdir(parents=True)
        (project / "src" / "alpha.rs").write_text(
            "//! alpha\nfn alpha() {}\n", encoding="utf-8"
        )
        build = runner.action_index_files_v2(
            project_root=str(project),
            repo_hash=REPO_HASH,
            worktree_hash=WT_BUILT,
            mode="full",
            db_root=self.db_root,
            scope="files",
        )
        self.assertTrue(build.get("ok"), build)

        payload = runner.action_status_v2(
            REPO_HASH,
            None,
            db_root=self.db_root,
            worktree_hashes=[WT_BUILT, WT_MISSING],
        )

        self.assertTrue(payload.get("ok"), payload)
        worktrees = payload.get("worktrees") or {}
        self.assertIn(WT_BUILT, worktrees, payload)
        self.assertIn(WT_MISSING, worktrees, payload)
        built_files = worktrees[WT_BUILT]["files"]
        self.assertTrue(built_files["healthy"], built_files)
        self.assertEqual(built_files["document_count"], 1, built_files)
        missing_files = worktrees[WT_MISSING]["files"]
        self.assertFalse(missing_files["exists"], missing_files)
        self.assertTrue(missing_files["repair_required"], missing_files)
        # Repo-shared scopes stay on the top-level status (compat, FR-398).
        self.assertIn("issues", payload.get("status") or {}, payload)


if __name__ == "__main__":
    unittest.main()
