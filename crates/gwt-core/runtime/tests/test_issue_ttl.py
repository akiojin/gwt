"""Phase 8: tests for the Issue index TTL handling.

The Issue index records last_full_refresh in meta.json. The status action
returns the remaining TTL. With --respect-ttl, refreshes within the TTL window
are no-ops.
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

    def test_index_issues_v2_skips_within_ttl_when_respect_ttl(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            issues = db_root / "abc1234567890def" / "issues"
            issues.mkdir(parents=True)
            now = datetime.datetime.now(datetime.timezone.utc)
            recent = now - datetime.timedelta(minutes=5)
            (issues / "meta.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "last_full_refresh": recent.isoformat(),
                        "ttl_minutes": 15,
                    }
                )
            )

            with mock.patch("subprocess.run") as gh:
                result = runner.action_index_issues_v2(
                    repo_hash="abc1234567890def",
                    project_root=tmp,
                    db_root=db_root,
                    respect_ttl=True,
                )

            self.assertTrue(result["ok"], result)
            self.assertTrue(result.get("skipped"))
            gh.assert_not_called()

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
