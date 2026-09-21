"""Phase 8: tests for the Issue index TTL handling.

The Issue index records last_full_refresh in meta.json. The status action
returns the remaining TTL. With --respect-ttl, only healthy refreshes within
the TTL window are no-ops. Failed rebuilds stop until repair or source change.
"""

from __future__ import annotations

import datetime
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


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

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
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
            with mock.patch.object(runner, "_load_cached_issue_documents", wraps=runner._load_cached_issue_documents) as load:
                recovered = runner.action_index_issues_v2(**args, respect_ttl=True)
            self.assertEqual(recovered.get("indexed"), 2, recovered)
            self.assertEqual(load.call_count, 1)

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
            self.assertEqual(health["reason"], "empty_corpus")
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

    def test_interrupted_build_leaves_marker_and_stops_same_source(self):
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
            with mock.patch.object(runner, "_make_chroma_collection_repairing") as create:
                self.assertEqual(runner.action_index_issues_v2(**args)["error_code"], "REPAIR_STOPPED")
            create.assert_not_called()
            self.assertTrue(runner.action_index_issues_v2(**args, repair=True)["ok"])
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
