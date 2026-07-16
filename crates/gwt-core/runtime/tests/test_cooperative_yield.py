"""Phase 70 T-IDX-386 (Issue #3264): cooperative yield cross-process contract.

FR-389: background embedding checkpoints at most every 16 documents. When a
higher-priority claimant is pending on the host-wide heavy lease, the
background build yields at the batch boundary, leaves a resumable
continuation, keeps the previously active index intact (AS-5), and a
follow-up run resumes without re-embedding already-staged documents.

The runner is exercised as a real subprocess (cross-process fidelity on both
Windows and POSIX): the pending heavy claimant is injected through the
coordinator directory (`GWT_INDEX_COORDINATOR_ROOT`).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

import chroma_index_runner as runner

RUNNER_PATH = Path(runner.__file__).resolve()

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"
TOTAL_DOCS = 40
CHECKPOINT_BATCH = 16


def _write_pending_claimant(coordinator_root: Path, priority: str) -> Path:
    pending_dir = coordinator_root / "heavy.pending"
    pending_dir.mkdir(parents=True, exist_ok=True)
    path = pending_dir / f"{uuid.uuid4()}.json"
    path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "owner": {"pid": 999999, "start_id": "test-claimant"},
                "priority": priority,
                "registered_at_ms": 0,
            }
        ),
        encoding="utf-8",
    )
    return path


class CooperativeYieldTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        base = Path(self._tmp.name)
        self.home = base / "home"
        self.home.mkdir()
        self.coordinator_root = base / "coordinator"
        self.coordinator_root.mkdir()
        self.project_root = base / "project"
        src = self.project_root / "src"
        src.mkdir(parents=True)
        for index in range(TOTAL_DOCS):
            (src / f"module_{index:02}.rs").write_text(
                f"//! module {index}\nfn feature_{index}() {{}}\n",
                encoding="utf-8",
            )
        self.db_root = self.home / ".gwt" / "index"

    def tearDown(self):
        self._tmp.cleanup()

    def _run_index(self) -> dict:
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        env["USERPROFILE"] = str(self.home)
        env["GWT_INDEX_FAKE_EMBEDDING"] = "1"
        env["GWT_INDEX_COORDINATOR_ROOT"] = str(self.coordinator_root)
        proc = subprocess.run(
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
                str(self.project_root),
                "--mode",
                "full",
                "--scope",
                "files",
                "--qos",
                "background",
            ],
            capture_output=True,
            text=True,
            timeout=180,
            env=env,
        )
        self.assertEqual(
            proc.returncode,
            0,
            f"runner failed: stdout={proc.stdout!r} stderr={proc.stderr!r}",
        )
        lines = [line for line in proc.stdout.splitlines() if line.strip()]
        self.assertTrue(lines, f"runner produced no stdout payload: {proc.stderr!r}")
        return json.loads(lines[-1])

    def _files_status(self) -> dict:
        return runner._scope_status_v2(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )

    def test_background_build_yields_at_checkpoint_and_resumes_from_staging(self):
        # 1. Baseline: a full build completes and becomes the active index.
        baseline = self._run_index()
        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("indexed"), TOTAL_DOCS, baseline)
        status = self._files_status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], TOTAL_DOCS, status)

        # 2. An interactive claimant is pending on the heavy lease: the
        #    background rebuild must yield at the 16-document boundary with a
        #    resumable continuation instead of finishing the batch. Every
        #    document is changed first so the rebuild has real embedding work
        #    (unchanged records are reused without checkpoints, FR-391).
        for index in range(TOTAL_DOCS):
            (self.project_root / "src" / f"module_{index:02}.rs").write_text(
                f"//! module {index} v2\nfn feature_{index}_v2() {{}}\n",
                encoding="utf-8",
            )
        pending = _write_pending_claimant(self.coordinator_root, "interactive-search")
        yielded = self._run_index()
        self.assertTrue(yielded.get("ok"), yielded)
        self.assertTrue(
            yielded.get("yielded"),
            f"background build must yield to the pending interactive claimant: {yielded}",
        )
        self.assertTrue(yielded.get("resumable"), yielded)
        self.assertEqual(
            yielded.get("newly_embedded"),
            CHECKPOINT_BATCH,
            f"yield must happen at the 16-document checkpoint boundary: {yielded}",
        )

        # 3. AS-5 / FR-390: the previously active index stays intact and
        #    searchable while the rebuild is parked in staging.
        status = self._files_status()
        self.assertTrue(
            status["healthy"],
            f"active index must stay healthy after a yielded rebuild: {status}",
        )
        self.assertEqual(
            status["document_count"],
            TOTAL_DOCS,
            f"active index must keep serving all documents after a yield: {status}",
        )

        # 4. Once the higher-priority claimant is gone, the rebuild resumes
        #    from the staged continuation and only embeds the remainder.
        pending.unlink()
        resumed = self._run_index()
        self.assertTrue(resumed.get("ok"), resumed)
        self.assertFalse(resumed.get("yielded"), resumed)
        self.assertEqual(resumed.get("indexed"), TOTAL_DOCS, resumed)
        self.assertEqual(
            resumed.get("newly_embedded"),
            TOTAL_DOCS - CHECKPOINT_BATCH,
            f"resume must not re-embed already-staged documents: {resumed}",
        )
        status = self._files_status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], TOTAL_DOCS, status)

    def test_equal_priority_pending_does_not_preempt_background_build(self):
        _write_pending_claimant(self.coordinator_root, "background")
        payload = self._run_index()
        self.assertTrue(payload.get("ok"), payload)
        self.assertFalse(
            payload.get("yielded"),
            f"equal-priority claimants must not preempt the running build: {payload}",
        )
        self.assertEqual(payload.get("indexed"), TOTAL_DOCS, payload)


if __name__ == "__main__":
    unittest.main()
