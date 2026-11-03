"""Phase 8: tests for the Issue index TTL handling.

The Issue index records last_full_refresh in meta.json. The status action
returns the remaining TTL. With --respect-ttl, only healthy refreshes within
the TTL window are no-ops. Failed rebuilds stop until repair or source change.
"""

from __future__ import annotations

import datetime
import json
import multiprocessing
import os
import tempfile
import time
import unittest
import uuid
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


def _hold_issue_build(db: str, fingerprint: str, started):
    with runner.acquire_lock(Path(db) / "build", exclusive=True):
        runner._issue_build_failure(Path(db), fingerprint, "full", "BUILD_INCOMPLETE", 0, 1)
        started.set()
        time.sleep(60)  # Parent kills this owned process after observing readiness.


class IssueTtlTests(unittest.TestCase):
    def _write_cached_issue(self, root: Path, number: int, title: str, body: str, labels):
        issue = root / str(number)
        issue.mkdir(parents=True, exist_ok=True)
        (issue / "meta.json").write_text(
            json.dumps(
                {
                    "number": number,
                    "title": title,
                    "labels": labels,
                    "state": "open",
                    "updated_at": "2026-04-13T00:00:00Z",
                    "comment_ids": [],
                }
            )
        )
        (issue / "body.md").write_text(body)

    def test_index_issues_v2_writes_meta_last_full_refresh(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(cache_root, 1, "First issue", "Body of issue 1", ["bug"])
            revision = str(uuid.uuid4())
            (cache_root / "source-revision").write_text(revision, encoding="utf-8")

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                legacy_fingerprint = runner._issue_source_fingerprint(
                    runner._load_cached_issue_documents("abc1234567890def"))
                result = runner.action_index_issues_v2(
                    repo_hash="abc1234567890def",
                    project_root=tmp,
                    db_root=db_root,
                    respect_ttl=False,
                )
            self.assertTrue(result["ok"], result)

            meta_path = (
                db_root / "abc1234567890def" / "issues" / "meta.json"
            )
            self.assertTrue(meta_path.exists())
            meta = json.loads(meta_path.read_text())
            self.assertIn("last_full_refresh", meta)
            self.assertEqual(meta.get("ttl_minutes"), 15)
            self.assertRegex(meta.get("source_cache_fingerprint", ""), r"^[0-9a-f]{64}$")
            self.assertEqual(meta.get("source_document_count"), 1)
            self.assertEqual(meta.get("source_cache_revision"), revision)
            self.assertEqual(meta.get("issue_cache_fingerprint"), legacy_fingerprint)

    def test_build_does_not_stamp_records_with_a_revision_changed_during_read(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            repo = "abc1234567890def"
            cache = root / ".gwt/cache/issues" / repo
            self._write_cached_issue(cache, 1, "First issue", "Body", ["bug"])
            stamp = cache / "source-revision"
            stamp.write_text(str(uuid.uuid4()), encoding="utf-8")
            load_records = runner._load_issue_records

            def load_then_advance(repo_hash):
                records = load_records(repo_hash)
                stamp.write_text(str(uuid.uuid4()), encoding="utf-8")
                return records

            with mock.patch.object(runner, "_load_issue_records", side_effect=load_then_advance):
                result = runner.action_index_issues_v2(repo, tmp, db_root=root / "index")
            self.assertTrue(result["ok"], result)
            meta = runner._read_issue_meta(root / "index" / repo / "issues")
            self.assertNotIn("source_cache_revision", meta)
            self.assertEqual(meta.get("issue_cache_fingerprint"), runner._issue_source_fingerprint(
                runner._load_cached_issue_documents(repo)))

    def test_status_reports_source_cache_changed_when_cached_issue_state_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(cache_root, 2867, "Recent Projects", "cache body", ["bug"])

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_index_issues_v2(
                    repo_hash="abc1234567890def",
                    project_root=tmp,
                    db_root=db_root,
                    respect_ttl=False,
                )
            self.assertTrue(result["ok"], result)

            meta_path = cache_root / "2867" / "meta.json"
            meta = json.loads(meta_path.read_text())
            meta["state"] = "closed"
            meta_path.write_text(json.dumps(meta))

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                status = runner.action_status_v2(
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    db_root=db_root,
                )

            issues = status["status"]["issues"]
            self.assertFalse(issues["healthy"], issues)
            self.assertTrue(issues["repair_required"], issues)
            self.assertEqual(issues["reason"], "source_cache_changed")
            self.assertTrue(issues["source_drift"], issues)

    def test_status_reports_count_mismatch_when_issue_cache_outgrows_index(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            issues_root = db_root / "abc1234567890def" / "issues"
            issues_root.mkdir(parents=True)
            (issues_root / runner.META_FILENAME).write_text("{}")
            indexed_meta = {
                "record_layout": runner.ISSUE_RECORD_LAYOUT,
                "document_count": 1,
                "source_document_count": 1,
                "source_cache_fingerprint": "indexed",
            }
            current_source = {"document_count": 2, "fingerprint": "current"}

            with (
                mock.patch.object(
                    runner, "_read_issue_meta", return_value=indexed_meta
                ),
                mock.patch.object(
                    runner,
                    "_issue_cache_source_snapshot",
                    return_value=current_source,
                ),
                mock.patch.object(
                    runner, "_scope_document_count", return_value=(True, 1)
                ),
            ):
                issues = runner._issue_status_v2(
                    repo_hash="abc1234567890def",
                    db_root=db_root,
                )

            self.assertFalse(issues["healthy"], issues)
            self.assertTrue(issues["repair_required"], issues)
            self.assertEqual(issues["reason"], "count_mismatch")
            self.assertEqual(issues["document_count"], 1)
            self.assertEqual(issues["source_document_count"], 1)
            self.assertEqual(issues["current_source_document_count"], 2)
            # Issue #4132: the store matches its own manifest and only trails
            # the Issue cache, so search must serve it instead of blocking.
            self.assertTrue(issues["source_drift"], issues)

    def test_status_reports_no_source_drift_when_store_contradicts_its_meta(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            issues_root = db_root / "abc1234567890def" / "issues"
            issues_root.mkdir(parents=True)
            (issues_root / runner.META_FILENAME).write_text("{}")
            indexed_meta = {
                "record_layout": runner.ISSUE_RECORD_LAYOUT,
                "document_count": 3,
                "source_document_count": 3,
                "source_cache_fingerprint": "indexed",
            }
            current_source = {"document_count": 3, "fingerprint": "indexed"}

            with (
                mock.patch.object(
                    runner, "_read_issue_meta", return_value=indexed_meta
                ),
                mock.patch.object(
                    runner,
                    "_issue_cache_source_snapshot",
                    return_value=current_source,
                ),
                mock.patch.object(
                    runner, "_scope_document_count", return_value=(True, 1)
                ),
            ):
                issues = runner._issue_status_v2(
                    repo_hash="abc1234567890def",
                    db_root=db_root,
                )

            self.assertFalse(issues["healthy"], issues)
            self.assertTrue(issues["repair_required"], issues)
            self.assertEqual(issues["reason"], "count_mismatch")
            self.assertFalse(issues["source_drift"], issues)

    def test_search_issues_rebuilds_after_cached_issue_state_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(cache_root, 2867, "Recent Projects", "cache body", ["bug"])

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                result = runner.action_index_issues_v2(
                    repo_hash="abc1234567890def",
                    project_root=tmp,
                    db_root=db_root,
                    respect_ttl=False,
                )
            self.assertTrue(result["ok"], result)

            meta_path = cache_root / "2867" / "meta.json"
            meta = json.loads(meta_path.read_text())
            meta["state"] = "closed"
            meta_path.write_text(json.dumps(meta))

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                search = runner.action_search_v2(
                    action="search-issues",
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    project_root=tmp,
                    query="Recent Projects",
                    n_results=5,
                    no_auto_build=False,
                    db_root=db_root,
                )

            self.assertTrue(search["ok"], search)
            self.assertEqual(search["issueResults"][0]["number"], 2867)
            self.assertEqual(search["issueResults"][0]["state"], "closed")

    def test_status_v2_returns_ttl_remaining_seconds(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            issues = db_root / "abc1234567890def" / "issues"
            issues.mkdir(parents=True)
            now = datetime.datetime.now(datetime.timezone.utc)
            five_minutes_ago = now - datetime.timedelta(minutes=5)
            (issues / "meta.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "last_full_refresh": five_minutes_ago.isoformat(),
                        "ttl_minutes": 15,
                    }
                )
            )

            result = runner.action_status_v2(
                repo_hash="abc1234567890def",
                worktree_hash=None,
                db_root=db_root,
            )
            self.assertTrue(result["ok"], result)
            issues_status = result["status"]["issues"]
            self.assertTrue(issues_status["exists"])
            remaining = issues_status["ttl_remaining_seconds"]
            self.assertGreater(remaining, 9 * 60)
            self.assertLess(remaining, 11 * 60)

    def test_index_issues_v2_skips_only_healthy_index_within_ttl(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            cache = root / ".gwt/cache/issues/abc1234567890def"
            self._write_cached_issue(cache, 1, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            self.assertTrue(runner.action_index_issues_v2(**args, respect_ttl=True)["skipped"])
            self._write_cached_issue(cache, 2, "Second", "body", [])
            result = runner.action_index_issues_v2(**args, respect_ttl=True)
            self.assertEqual(result.get("indexed"), 2, result)
            db = root / "index/abc1234567890def/issues"
            client, collection = runner._open_chroma_collection(runner.resolve_active_store(db), "issues")
            try:
                collection.delete(ids=["1", "2"])
            finally:
                runner._close_chroma_client(client)
            recovered = runner.action_index_issues_v2(**args, respect_ttl=True)
            self.assertEqual(recovered.get("indexed"), 2, recovered)

    def test_empty_cache_is_failure_and_explicit_repair_recovers(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            result = runner.action_index_issues_v2(**args)
            self.assertFalse(result["ok"], result)
            self.assertEqual(result["error_code"], "EMPTY_CORPUS")
            self.assertFalse((root / "index/abc1234567890def/issues/meta.json").exists())
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "First", "body", [])
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])

    def test_legacy_empty_index_cannot_skip_within_ttl(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            db = root / "index/abc1234567890def/issues"
            client, _ = runner._make_chroma_collection_repairing(db, "issues")
            runner._close_chroma_client(client)
            runner._write_issue_meta(db, {
                "document_count": 0, "source_document_count": 0,
                "source_cache_fingerprint": runner._issue_source_fingerprint([]),
                "last_full_refresh": runner._now_utc().isoformat(), "ttl_minutes": 15,
            })
            health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
            self.assertFalse(health["healthy"], health)
            self.assertEqual(health["reason"], "record_layout_changed")
            result = runner.action_index_issues_v2(**args, respect_ttl=True)
            self.assertFalse(result["ok"], result)
            self.assertEqual(result["error_code"], "EMPTY_CORPUS")

    def test_count_failure_stops_repeat_and_explicit_repair_resets(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            self.assertTrue(runner.action_index_issues_v2(**args, mode="incremental")["ok"])
            for actual in (0, 2):
                with mock.patch.object(runner, "_safe_collection_count", return_value=actual):
                    result = runner.action_index_issues_v2(**args, repair=True)
                self.assertFalse(result["ok"], result)
                self.assertEqual(result["error_code"], "COUNT_MISMATCH")
                with mock.patch.object(runner, "_make_chroma_collection_repairing") as create:
                    stopped = runner.action_index_issues_v2(**args)
                self.assertEqual(stopped["error_code"], "REPAIR_STOPPED")
                create.assert_not_called()
                health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
                self.assertEqual(health["reason"], "repair_stopped")
                self.assertEqual(health["mode"], "full")
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 2, "Second", "body", [])
            health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
            self.assertNotEqual(health["reason"], "repair_stopped")
            self.assertEqual(health["mode"], "incremental")
            self.assertTrue(runner.action_index_issues_v2(**args, repair=True)["ok"])

    def test_published_count_mismatch_stops_next_build_and_status(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            cache = root / ".gwt/cache/issues/abc1234567890def"
            self._write_cached_issue(cache, 1, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            db = root / "index/abc1234567890def/issues"
            publish = runner._publish_generation

            def publish_then_lose_records(*args, **kwargs):
                result = publish(*args, **kwargs)
                client, collection = runner._open_chroma_collection(runner.resolve_active_store(db), "issues")
                try:
                    collection.delete(ids=["1"])
                finally:
                    runner._close_chroma_client(client)
                return result

            with mock.patch.object(runner, "_publish_generation", side_effect=publish_then_lose_records):
                result = runner.action_index_issues_v2(**args)
            self.assertFalse(result["ok"], result)
            self.assertEqual(result["error_code"], "COUNT_MISMATCH")
            health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
            self.assertEqual(health["reason"], "repair_stopped")
            self.assertFalse(health["healthy"])
            with mock.patch.object(runner, "_make_chroma_collection_repairing") as create:
                self.assertEqual(runner.action_index_issues_v2(**args)["error_code"], "REPAIR_STOPPED")
            create.assert_not_called()
            self._write_cached_issue(cache, 2, "Second", "body", [])
            health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
            self.assertNotEqual(health["reason"], "repair_stopped")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])

    def test_publication_exceptions_stop_repeat_build(self):
        for failing_operation in ("_publish_generation", "write_manifest"):
            with self.subTest(operation=failing_operation), tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
                root = Path(tmp)
                self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "First", "body", [])
                args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
                with mock.patch.object(runner, failing_operation, side_effect=RuntimeError("publish interrupted")):
                    result = runner.action_index_issues_v2(**args)
                self.assertFalse(result["ok"], result)
                self.assertEqual(result["error_code"], "PUBLISH_FAILED")
                with mock.patch.object(runner, "_make_chroma_collection_repairing") as create:
                    self.assertEqual(runner.action_index_issues_v2(**args)["error_code"], "REPAIR_STOPPED")
                create.assert_not_called()

    def test_publication_os_error_preserves_active_generation_and_diagnostic(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            cache = root / ".gwt/cache/issues/abc1234567890def"
            self._write_cached_issue(cache, 1, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            db = runner.resolve_db_path(args["repo_hash"], None, "issues", db_root=args["db_root"])
            pointer_before = runner.active_pointer_path(db).read_bytes()
            meta_before = (db / runner.META_FILENAME).read_bytes()
            replace = os.replace
            error = PermissionError("[WinError 5] staging handle is still open")

            def fail_staging(source, destination):
                if Path(source) == runner._staging_dir_for(db):
                    raise error
                return replace(source, destination)

            with mock.patch.object(runner.os, "replace", side_effect=fail_staging):
                result = runner.action_index_issues_v2(**args, mode="incremental")
            self.assertEqual(result["error_code"], "PUBLISH_FAILED", result)
            self.assertIn(str(error), result["error"])
            self.assertIn(str(error), runner._read_issue_repair(db)["error"])
            self.assertEqual(runner.active_pointer_path(db).read_bytes(), pointer_before)
            self.assertEqual((db / runner.META_FILENAME).read_bytes(), meta_before)

    @unittest.skipUnless(os.name == "nt", "Windows native publication contract")
    def test_native_incremental_refresh_publishes_1622_issues_after_ttl(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {
            "HOME": tmp, "GWT_INDEX_COORDINATOR_ROOT": str(Path(tmp) / "coordinator")
        }):
            issues = [{"number": number, "title": f"Issue {number}", "body": "body",
                       "state": "open", "labels": []} for number in range(1, 1623)]
            fake = runner._FakeEmbeddingModel()
            model = mock.Mock()
            # Keep native Chroma/SQLx/HNSW, with the production E5 vector size.
            model.encode.side_effect = lambda texts: [vector * 24 for vector in fake.encode(texts)]
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=Path(tmp) / "index")
            with mock.patch.object(runner, "_load_cached_issue_documents", return_value=issues), mock.patch.object(
                runner, "_get_embedding_model", return_value=model
            ), mock.patch.object(runner, "emit_progress"):
                baseline = runner.action_index_issues_v2(**args)
                self.assertTrue(baseline["ok"], baseline)
                db = runner.resolve_db_path(args["repo_hash"], None, "issues", db_root=args["db_root"])
                pointer_before = runner.active_pointer_path(db).read_bytes()
                meta = runner._read_issue_meta(db)
                meta["last_full_refresh"] = "2000-01-01T00:00:00Z"
                runner._write_issue_meta(db, meta)
                issues[0]["body"] = "updated body"
                issues[-1] = dict(issues[-1], number=1623)
                result = runner.action_index_issues_v2(**args, mode="incremental", respect_ttl=True)
                self.assertTrue(result["ok"], result)
                self.assertEqual(result["mode"], "incremental")
                self.assertEqual(result["indexed"], 1622)
                self.assertEqual(result["newly_embedded"], 2)
                self.assertNotEqual(runner.active_pointer_path(db).read_bytes(), pointer_before)
                self.assertTrue(runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])["healthy"])

    def test_building_serves_previous_generation(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            cache = root / ".gwt/cache/issues/abc1234567890def"
            self._write_cached_issue(cache, 1, "Published", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            self._write_cached_issue(cache, 1, "Pending", "body changed", [])
            create = runner._make_chroma_collection_repairing

            def inspect_active_build(*create_args, **kwargs):
                health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
                self.assertEqual(health["reason"], "building", health)
                result = runner.action_search_multi_v2(
                    **args, worktree_hash=None, query="Published", n_results=5, scopes=["issues"])
                self.assertTrue(result["ok"], result)
                self.assertEqual(result["scopes"]["issues"]["state"], "stale", result)
                self.assertEqual(result["issueResults"][0]["title"], "Published")
                direct = runner.action_search_v2(
                    **args, action="search-issues", worktree_hash=None,
                    query="Published", n_results=5, no_auto_build=True)
                self.assertTrue(direct["ok"], direct)
                self.assertEqual(direct["issueResults"][0]["title"], "Published")
                return create(*create_args, **kwargs)

            with mock.patch.object(runner, "_make_chroma_collection_repairing", side_effect=inspect_active_build):
                self.assertTrue(runner.action_index_issues_v2(**args)["ok"])

    def test_build_started_after_recovery_probe_is_building(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "Published", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            db = root / "index/abc1234567890def/issues"
            fingerprint = runner._read_issue_meta(db)["source_cache_fingerprint"]

            def start_after_probe(_db):
                runner._issue_build_failure(db, fingerprint, "full", "BUILD_INCOMPLETE", 0, 1)
                return False

            with mock.patch.object(runner, "_recover_issue_build", side_effect=start_after_probe):
                health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
            self.assertEqual(health["reason"], "building", health)
            self.assertTrue(health["healthy"], health)

    def test_status_recovers_sentinel_after_builder_process_dies(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "Published", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            db = root / "index/abc1234567890def/issues"
            fingerprint = runner._read_issue_meta(db)["source_cache_fingerprint"]
            ctx = multiprocessing.get_context("spawn")
            started = ctx.Event()
            process = ctx.Process(target=_hold_issue_build, args=(str(db), fingerprint, started))
            process.start()
            try:
                self.assertTrue(started.wait(30), "builder did not acquire its lock")
                self.assertEqual(runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])["reason"], "building")
                process.terminate()
                process.join(30)
                self.assertFalse(process.is_alive())
                health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
                self.assertTrue(health["healthy"], health)
                self.assertFalse((db / "repair.json").exists())
            finally:
                if process.is_alive():
                    process.terminate()
                process.join(30)

    def test_interrupted_build_leaves_marker_and_next_build_recovers(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            with mock.patch.object(runner, "_make_chroma_collection_repairing", side_effect=KeyboardInterrupt):
                with self.assertRaises(KeyboardInterrupt):
                    runner.action_index_issues_v2(**args)
            db = root / "index/abc1234567890def/issues"
            marker = runner._read_issue_repair(db)
            self.assertEqual(marker.get("last_error"), "BUILD_INCOMPLETE")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            self.assertFalse((db / "repair.json").exists())

    def test_incremental_reuses_unchanged_and_removes_deleted(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            cache = root / ".gwt/cache/issues/abc1234567890def"
            for number in (1, 2, 3):
                self._write_cached_issue(cache, number, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            self.assertEqual(runner.action_index_issues_v2(**args)["newly_embedded"], 3)
            self._write_cached_issue(cache, 2, "Changed", "new body", [])
            import shutil
            shutil.rmtree(cache / "3")
            self._write_cached_issue(cache, 4, "Added", "body", [])
            result = runner.action_index_issues_v2(**args, mode="incremental")
            self.assertEqual(result.get("mode"), "incremental", result)
            self.assertEqual(result["newly_embedded"], 2)
            db = root / "index/abc1234567890def/issues"
            client, collection = runner._open_chroma_collection(runner.resolve_active_store(db), "issues")
            try:
                records = collection.get()
                self.assertEqual(set(records["ids"]), {"1", "2", "4"})
                self.assertIn("Changed\nnew body", records["documents"])
            finally:
                runner._close_chroma_client(client)

    def test_spec_chunks_share_the_issue_store_and_filter_before_top_k(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            repo = "abc1234567890def"
            cache = root / ".gwt/cache/issues" / repo
            self._write_cached_issue(cache, 1, "Shared alpha", "alpha issue", ["bug"])
            self._write_cached_issue(cache, 2, "Shared alpha", "body wrapper", ["GWT-SPEC"])
            sections = cache / "2/sections"
            sections.mkdir()
            spec = sections / "spec.md"
            spec.write_text("## Alpha\nalpha requirements\n## Omega\nomega requirements", encoding="utf-8")
            args = dict(repo_hash=repo, project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_specs_v2(worktree_hash=None, **args)["ok"])
            db = runner.resolve_db_path(repo, None, "issues", args["db_root"])
            self.assertNotIn("source_cache_revision", runner._read_issue_meta(db))
            self.assertEqual(runner.resolve_db_path(repo, None, "specs", args["db_root"]), db)
            self.assertEqual(runner.read_manifest(db, "specs"), runner.read_manifest(db, "issues"))
            client, collection = runner._open_chroma_collection(runner.resolve_active_store(db), "issues")
            try:
                records = collection.get()
                self.assertEqual(set(records["ids"]), {"1", "spec-2:chunk-0", "spec-2:chunk-1"})
                self.assertEqual(sorted(meta["record_kind"] for meta in records["metadatas"]), ["issue", "spec", "spec"])
            finally:
                runner._close_chroma_client(client)
            result = runner.action_search_multi_v2(
                repo, None, tmp, "alpha", 1, ["specs", "issues"], db_root=args["db_root"])
            self.assertEqual(result["specResults"][0]["spec_id"], "2", result)
            self.assertEqual(result["issueResults"][0]["number"], 1, result)
            self.assertTrue(all(scope["state"] == "fresh" for scope in result["scopes"].values()), result)

            # A section edit outside the truncated Issue body must invalidate
            # the shared source, then reuse the unchanged issue and chunk.
            spec.write_text("## Alpha\nalpha requirements\n## Omega\nchanged requirements", encoding="utf-8")
            self.assertTrue(runner._issue_status_v2(repo, args["db_root"])["source_drift"])
            rebuilt = runner.action_index_issues_v2(**args, mode="incremental")
            self.assertEqual(rebuilt["newly_embedded"], 1, rebuilt)
            self._write_cached_issue(cache, 2, "Plain now", "ordinary body", [])
            rebuilt = runner.action_index_issues_v2(**args, mode="incremental")
            self.assertEqual(rebuilt["newly_embedded"], 1, rebuilt)
            result = runner.action_search_multi_v2(
                repo, None, tmp, "alpha", 5, ["specs", "issues"], db_root=args["db_root"])
            self.assertEqual(result["specResults"], [], result)
            self.assertEqual({item["number"] for item in result["issueResults"]}, {1, 2}, result)

    def test_old_issue_layout_is_not_served_as_a_unified_store(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            repo = "abc1234567890def"
            self._write_cached_issue(root / ".gwt/cache/issues" / repo, 1, "First", "body", [])
            args = dict(repo_hash=repo, project_root=tmp, db_root=root / "index")
            self.assertTrue(runner.action_index_issues_v2(**args)["ok"])
            db = runner.resolve_db_path(repo, None, "issues", args["db_root"])
            meta = runner._read_issue_meta(db)
            meta.pop("record_layout", None)
            runner._write_issue_meta(db, meta)
            for action in ("search-issues", "search-specs"):
                result = runner.action_search_v2(
                    action=action, worktree_hash=None, query="First", no_auto_build=True, **args)
                self.assertFalse(result["ok"], result)
                self.assertEqual(result["status"]["reason"], "record_layout_changed")
            rebuilt = runner.action_index_issues_v2(**args, mode="incremental", respect_ttl=True)
            self.assertTrue(rebuilt["ok"], rebuilt)
            self.assertEqual(rebuilt["mode"], "full", rebuilt)

    def test_cancel_prevents_publish_and_explicit_repair_clears_flag(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp)
            self._write_cached_issue(root / ".gwt/cache/issues/abc1234567890def", 1, "First", "body", [])
            args = dict(repo_hash="abc1234567890def", project_root=tmp, db_root=root / "index")
            db = root / "index/abc1234567890def/issues"
            db.mkdir(parents=True)
            (db / "cancel-requested").touch()
            health = runner._issue_status_v2(args["repo_hash"], db_root=args["db_root"])
            self.assertEqual(health["reason"], "cancelled")
            self.assertFalse(health["healthy"])
            self.assertEqual(runner.action_index_issues_v2(**args)["error_code"], "CANCELLED")
            self.assertFalse((db / "meta.json").exists())
            with mock.patch.object(runner, "_write_heavy_progress", side_effect=lambda *args: (db / "cancel-requested").touch()):
                cancelled = runner.action_index_issues_v2(**args, repair=True)
            self.assertEqual(cancelled["error_code"], "CANCELLED")
            self.assertFalse((db / "meta.json").exists())
            self.assertTrue(runner.action_index_issues_v2(**args, repair=True)["ok"])
            self.assertFalse((db / "cancel-requested").exists())

    def test_index_issues_v2_runs_when_ttl_expired(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            issues = db_root / "abc1234567890def" / "issues"
            issues.mkdir(parents=True)
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(
                cache_root,
                1,
                "First issue",
                "Body of issue 1",
                ["bug"],
            )
            now = datetime.datetime.now(datetime.timezone.utc)
            stale = now - datetime.timedelta(minutes=20)
            (issues / "meta.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "last_full_refresh": stale.isoformat(),
                        "ttl_minutes": 15,
                    }
                )
            )

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                with mock.patch("subprocess.run") as gh:
                    result = runner.action_index_issues_v2(
                        repo_hash="abc1234567890def",
                        project_root=tmp,
                        db_root=db_root,
                        respect_ttl=True,
                    )

            self.assertTrue(result["ok"], result)
            self.assertFalse(result.get("skipped"))
            gh.assert_not_called()

    def test_index_issues_v2_reads_repo_scoped_issue_cache_without_gh(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / "abc1234567890def"
            self._write_cached_issue(
                cache_root,
                1776,
                "Launch Agent issue linkage",
                "Body from cache",
                ["ux"],
            )

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                with mock.patch("subprocess.run") as gh:
                    result = runner.action_index_issues_v2(
                        repo_hash="abc1234567890def",
                        project_root=tmp,
                        db_root=db_root,
                        respect_ttl=False,
                    )

            self.assertTrue(result["ok"], result)
            self.assertFalse(
                any(call.args and call.args[0] == "gh" for call in gh.call_args_list),
                gh.call_args_list,
            )


if __name__ == "__main__":
    unittest.main()
