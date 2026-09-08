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
import time
import unittest
import uuid
from unittest import mock
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



def _write_reservation(coordinator_root: Path, priority: str, reserved_until_ms: int) -> Path:
    """Issue #4086: a durable pending claim with no live lock holder."""
    pending_dir = coordinator_root / "heavy.pending"
    pending_dir.mkdir(parents=True, exist_ok=True)
    path = pending_dir / "reservation-repo--verification--wt.json"
    path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "owner": {"pid": 999999, "start_id": "test-reservation"},
                "priority": priority,
                "registered_at_ms": 0,
                "reserved_until_ms": reserved_until_ms,
            }
        ),
        encoding="utf-8",
    )
    return path


class HeavyReservationTests(unittest.TestCase):
    """Issue #4086 AC-1 / AC-2: reservations are honored only while unexpired."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.coordinator_root = Path(self._tmp.name) / "coordinator"
        self.coordinator_root.mkdir()
        self._env = mock.patch.dict(
            os.environ, {"GWT_INDEX_COORDINATOR_ROOT": str(self.coordinator_root)}
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()

    def test_live_reservation_preempts_background(self):
        _write_reservation(
            self.coordinator_root, "manual-rebuild", int(time.time() * 1000) + 600_000
        )
        self.assertTrue(runner._pending_higher_priority("background"))

    def test_expired_reservation_is_ignored(self):
        _write_reservation(self.coordinator_root, "manual-rebuild", int(time.time() * 1000) - 1)
        self.assertFalse(runner._pending_higher_priority("background"))


ISSUE_REPO_HASH = "4086408640864086"
ISSUE_TOTAL = 40


class IssueIndexCooperativeYieldTests(unittest.TestCase):
    """Issue #4086 AC-2 / AC-4: the issues build checkpoints like the files
    build — it yields at the batch boundary to a pending verification
    claimant, resumes from staging, and publishes its batch progress next to
    the heavy ticket so a refused claimant can estimate the wait."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        base = Path(self._tmp.name)
        self.home = base / "home"
        self.home.mkdir()
        self.coordinator_root = base / "coordinator"
        self.coordinator_root.mkdir()
        self.db_root = self.home / ".gwt" / "index"
        cache_root = self.home / ".gwt" / "cache" / "issues" / ISSUE_REPO_HASH
        for number in range(1, ISSUE_TOTAL + 1):
            issue = cache_root / str(number)
            issue.mkdir(parents=True, exist_ok=True)
            (issue / "meta.json").write_text(
                json.dumps(
                    {
                        "number": number,
                        "title": f"Issue {number}",
                        "labels": ["bug"],
                        "state": "open",
                        "updated_at": "2026-09-07T00:00:00Z",
                        "comment_ids": [],
                    }
                ),
                encoding="utf-8",
            )
            (issue / "body.md").write_text(f"body of issue {number}\n", encoding="utf-8")
        self._env = mock.patch.dict(
            os.environ,
            {
                "HOME": str(self.home),
                "USERPROFILE": str(self.home),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
                "GWT_INDEX_COORDINATOR_ROOT": str(self.coordinator_root),
            },
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()

    def _run(self) -> dict:
        return runner.action_index_issues_v2(
            repo_hash=ISSUE_REPO_HASH,
            project_root=str(self.home),
            db_root=self.db_root,
            respect_ttl=False,
            qos="background",
        )

    def _progress(self) -> dict:
        return json.loads(
            (self.coordinator_root / "heavy.progress.json").read_text(encoding="utf-8")
        )

    def test_issue_build_yields_to_a_verification_reservation_and_resumes(self):
        pending = _write_reservation(
            self.coordinator_root, "manual-rebuild", int(time.time() * 1000) + 600_000
        )
        yielded = self._run()
        self.assertTrue(yielded.get("ok"), yielded)
        self.assertTrue(yielded.get("yielded"), yielded)
        self.assertTrue(yielded.get("resumable"), yielded)
        self.assertEqual(yielded.get("newly_embedded"), CHECKPOINT_BATCH, yielded)

        progress = self._progress()
        self.assertEqual(progress["target"], f"{ISSUE_REPO_HASH}--issues")
        self.assertEqual(progress["done"], CHECKPOINT_BATCH)
        self.assertEqual(progress["total"], ISSUE_TOTAL)
        self.assertEqual(progress["batch_size"], CHECKPOINT_BATCH)
        self.assertGreaterEqual(progress["batch_ms"], 0)

        pending.unlink()
        resumed = self._run()
        self.assertTrue(resumed.get("ok"), resumed)
        self.assertFalse(resumed.get("yielded"), resumed)
        self.assertEqual(resumed.get("indexed"), ISSUE_TOTAL, resumed)
        self.assertEqual(
            resumed.get("newly_embedded"),
            ISSUE_TOTAL - CHECKPOINT_BATCH,
            f"resume must not re-embed already-staged issues: {resumed}",
        )
        self.assertEqual(self._progress()["done"], ISSUE_TOTAL)

    def test_issue_build_completes_without_pending_claimants(self):
        payload = self._run()
        self.assertTrue(payload.get("ok"), payload)
        self.assertFalse(payload.get("yielded"), payload)
        self.assertEqual(payload.get("indexed"), ISSUE_TOTAL, payload)


if __name__ == "__main__":
    unittest.main()
