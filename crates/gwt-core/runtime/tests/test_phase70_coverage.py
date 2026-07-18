"""Phase 70 T-IDX-400 (Issue #3264): targeted coverage for changed branches.

Exercises the error / fallback / dispatch branches of the Phase 70 changes
that the behavioral suites reach only through subprocesses (which coverage
cannot trace) or not at all.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


class QosBranchTests(unittest.TestCase):
    def test_unknown_qos_profile_is_rejected(self):
        with self.assertRaises(ValueError):
            runner.configure_qos_threads("turbo")

    def test_torch_thread_errors_are_swallowed(self):
        fake_torch = types.ModuleType("torch")
        fake_torch.set_num_threads = mock.Mock(side_effect=RuntimeError("busy"))
        fake_torch.set_num_interop_threads = mock.Mock()
        with mock.patch.dict(sys.modules, {"torch": fake_torch}):
            runner.configure_qos_threads("background")

    @unittest.skipIf(os.name == "nt", "POSIX nice branch")
    def test_nice_failure_is_swallowed(self):
        with mock.patch("os.nice", side_effect=OSError(1, "denied")):
            runner.configure_qos_threads("background")


class CoordinatorLookupTests(unittest.TestCase):
    def test_coordinator_root_falls_back_to_home(self):
        env = {"HOME": "/tmp/gwt-test-home"}
        with mock.patch.dict(os.environ, env, clear=False):
            os.environ.pop("GWT_INDEX_COORDINATOR_ROOT", None)
            root = runner._coordinator_root()
        self.assertEqual(
            root,
            Path("/tmp/gwt-test-home") / ".gwt" / "runtime" / "index-coordinator",
        )

    def test_pending_scan_skips_non_json_and_corrupt_entries(self):
        with tempfile.TemporaryDirectory() as tmp:
            pending = Path(tmp) / "heavy.pending"
            pending.mkdir()
            (pending / "note.txt").write_text("ignore", encoding="utf-8")
            (pending / "corrupt.json").write_text("{oops", encoding="utf-8")
            (pending / "low.json").write_text(
                json.dumps({"priority": "background"}), encoding="utf-8"
            )
            with mock.patch.dict(
                os.environ, {"GWT_INDEX_COORDINATOR_ROOT": tmp}, clear=False
            ):
                self.assertFalse(runner._pending_higher_priority("background"))
                (pending / "high.json").write_text(
                    json.dumps({"priority": "interactive-search"}), encoding="utf-8"
                )
                self.assertTrue(runner._pending_higher_priority("background"))


class GenerationBranchTests(unittest.TestCase):
    def test_read_continuation_tolerates_unreadable_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            # A directory raises OSError on read_text.
            self.assertIsNone(runner._read_continuation(Path(tmp)))

    def test_active_pointer_rejects_non_object_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "files"
            pointer = runner.active_pointer_path(db_path)
            pointer.parent.mkdir(parents=True)
            pointer.write_text("[1, 2, 3]", encoding="utf-8")
            self.assertIsNone(runner._read_active_pointer(db_path))
            self.assertTrue(runner._active_pointer_corrupt(db_path))

    def test_gc_tolerates_missing_generations_root(self):
        runner._gc_abandoned_generations(
            Path("/nonexistent/gwt-gen-root"), keep=set()
        )

    def test_remove_legacy_chroma_files_tolerates_missing_dir(self):
        runner._remove_legacy_chroma_files(Path("/nonexistent/gwt-db"))

    def test_copy_unchanged_records_swallows_previous_store_errors(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "files"
            db_path.mkdir()
            (db_path / "chroma.sqlite3").write_text("stub", encoding="utf-8")
            entries = [{"path": "a.rs", "content_hash": "h1"}]
            runner.write_manifest(db_path, scope="files", entries=entries)
            with mock.patch.object(
                runner, "_open_chroma_collection", side_effect=RuntimeError("broken")
            ):
                copied = runner._copy_unchanged_records(
                    db_path, "files", None, runner.V2_FILES_CODE_COLLECTION, {"a.rs": "h1"}
                )
            self.assertEqual(copied, 0)


class InProcessYieldTests(unittest.TestCase):
    def test_background_full_build_yields_in_process(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            coord = base / "coordinator"
            pending = coord / "heavy.pending"
            pending.mkdir(parents=True)
            (pending / "claimant.json").write_text(
                json.dumps({"priority": "interactive-search"}), encoding="utf-8"
            )
            project = base / "project"
            (project / "src").mkdir(parents=True)
            for index in range(20):
                (project / "src" / f"m_{index:02}.rs").write_text(
                    f"//! m {index}\nfn f_{index}() {{}}\n", encoding="utf-8"
                )
            with mock.patch.dict(
                os.environ,
                {
                    "GWT_INDEX_COORDINATOR_ROOT": str(coord),
                    "GWT_INDEX_FAKE_EMBEDDING": "1",
                },
                clear=False,
            ):
                result = runner.action_index_files_v2(
                    project_root=str(project),
                    repo_hash=REPO_HASH,
                    worktree_hash=WORKTREE_HASH,
                    mode="full",
                    db_root=base / "index",
                    scope="files",
                    qos="background",
                )
            self.assertTrue(result.get("ok"), result)
            self.assertTrue(result.get("yielded"), result)
            self.assertEqual(result.get("newly_embedded"), 16, result)


class PublishFailurePropagationTests(unittest.TestCase):
    def test_scope_actions_propagate_publish_failures(self):
        failure = {"ok": False, "error_code": "PUBLISH_FAILED", "error": "disk full"}
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = base / "project"
            project.mkdir()
            cases = [
                lambda: runner.action_index_specs_v2(
                    project_root=str(project),
                    repo_hash=REPO_HASH,
                    worktree_hash="",
                    mode="full",
                    db_root=db_root,
                ),
                lambda: runner.action_index_memory_v2(
                    project_root=str(project),
                    repo_hash=REPO_HASH,
                    worktree_hash=None,
                    mode="full",
                    db_root=db_root,
                ),
                lambda: runner.action_index_discussions_v2(
                    project_root=str(project),
                    repo_hash=REPO_HASH,
                    worktree_hash=None,
                    mode="full",
                    db_root=db_root,
                ),
                lambda: runner.action_index_board_v2(
                    repo_hash=REPO_HASH,
                    project_root=str(project),
                    mode="full",
                    db_root=db_root,
                ),
                lambda: runner.action_index_works_v2(
                    project_root=str(project),
                    repo_hash=REPO_HASH,
                    worktree_hash=None,
                    mode="full",
                    db_root=db_root,
                ),
            ]
            env = {
                "GWT_INDEX_FAKE_EMBEDDING": "1",
                "HOME": str(base),
                "USERPROFILE": str(base),
            }
            with mock.patch.dict(os.environ, env, clear=False):
                for case in cases:
                    with mock.patch.object(
                        runner, "_finish_full_build", return_value=failure
                    ):
                        result = case()
                    self.assertEqual(result, failure)
                with mock.patch.object(
                    runner, "_publish_generation", return_value=failure
                ):
                    result = runner.action_index_issues_v2(
                        repo_hash=REPO_HASH,
                        project_root=str(project),
                        db_root=db_root,
                        respect_ttl=False,
                    )
                self.assertEqual(result, failure)


class SearchClassificationBranchTests(unittest.TestCase):
    def test_issue_scope_classification_states(self):
        healthy_fresh = {
            "exists": True,
            "healthy": True,
            "ttl_remaining_seconds": 120,
        }
        stale_drift = {
            "exists": True,
            "healthy": False,
            "reason": "source_cache_changed",
        }
        broken_meta = {
            "exists": True,
            "healthy": False,
            "reason": "metadata_missing",
        }
        for health, expected in [
            (healthy_fresh, "fresh"),
            (stale_drift, "stale"),
            (broken_meta, "corrupt"),
        ]:
            with mock.patch.object(runner, "_issue_status_v2", return_value=health):
                state, _ = runner._classify_scope_for_search(
                    REPO_HASH, None, "issues", db_root=None
                )
            self.assertEqual(state, expected, health)

    def test_search_multi_marks_scope_corrupt_when_query_blows_up(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.dict(
                os.environ,
                {
                    "GWT_INDEX_FAKE_EMBEDDING": "1",
                    "GWT_INDEX_COORDINATOR_ROOT": tmp,
                },
                clear=False,
            ), mock.patch.object(
                runner,
                "_classify_scope_for_search",
                return_value=("fresh", {"reason": "ready"}),
            ), mock.patch.object(
                runner,
                "_search_scope_collection",
                side_effect=RuntimeError("hnsw exploded"),
            ):
                payload = runner.action_search_multi_v2(
                    repo_hash=REPO_HASH,
                    worktree_hash=None,
                    project_root=None,
                    query="q",
                    n_results=3,
                    scopes=["works"],
                    db_root=Path(tmp),
                )
        self.assertTrue(payload.get("ok"), payload)
        self.assertEqual(payload["scopes"]["works"]["state"], "corrupt", payload)

    def test_search_scope_collection_emits_all_terms_suggestions(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = base / "project"
            (project / "src").mkdir(parents=True)
            (project / "src" / "alpha.rs").write_text(
                "//! alpha module\nfn alpha() {}\n", encoding="utf-8"
            )
            with mock.patch.dict(
                os.environ,
                {
                    "GWT_INDEX_FAKE_EMBEDDING": "1",
                    "GWT_INDEX_COORDINATOR_ROOT": str(base / "coord"),
                },
                clear=False,
            ):
                build = runner.action_index_files_v2(
                    project_root=str(project),
                    repo_hash=REPO_HASH,
                    worktree_hash=WORKTREE_HASH,
                    mode="full",
                    db_root=db_root,
                    scope="files",
                )
                self.assertTrue(build.get("ok"), build)
                payload = runner._search_scope_collection(
                    REPO_HASH,
                    WORKTREE_HASH,
                    "files",
                    "alpha missingterm",
                    5,
                    "all_terms",
                    db_root,
                    None,
                )
            self.assertIn("suggestions", payload, payload)


class StatusDispatchTests(unittest.TestCase):
    def test_status_dispatch_parses_worktree_hashes_argument(self):
        with tempfile.TemporaryDirectory() as tmp:
            argv = [
                "chroma_index_runner.py",
                "--action",
                "status",
                "--repo-hash",
                REPO_HASH,
                "--worktree-hashes",
                f"{WORKTREE_HASH}, ",
                "--db-root",
                tmp,
            ]
            stdout = io.StringIO()
            with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(
                stdout
            ):
                exit_code = runner.main()
            self.assertEqual(exit_code, 0)
            payload = json.loads(stdout.getvalue())
            self.assertIn(WORKTREE_HASH, payload.get("worktrees") or {}, payload)


if __name__ == "__main__":
    unittest.main()
