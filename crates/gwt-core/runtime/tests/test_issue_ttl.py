"""Phase 8: tests for the Issue index TTL handling.

The Issue index records last_full_refresh in meta.json. The status action
returns the remaining TTL. With --respect-ttl, refreshes within the TTL window
are no-ops.
"""

from __future__ import annotations

import datetime
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class IssueTtlTests(unittest.TestCase):
    def _fake_gh_issue_list(self, *args, **kwargs):
        # subprocess.run mock returning a fake gh issue list result
        result = mock.MagicMock()
        result.returncode = 0
        result.stdout = json.dumps(
            [
                {
                    "number": 1,
                    "title": "First issue",
                    "body": "Body of issue 1",
                    "labels": [{"name": "bug"}],
                    "state": "OPEN",
                    "url": "https://github.com/example/repo/issues/1",
                }
            ]
        )
        result.stderr = ""
        return result

    def test_index_issues_v2_writes_meta_last_full_refresh(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"

            with mock.patch("subprocess.run", side_effect=self._fake_gh_issue_list):
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

            with mock.patch("subprocess.run", side_effect=self._fake_gh_issue_list) as gh:
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

            with mock.patch("subprocess.run", side_effect=self._fake_gh_issue_list) as gh:
                result = runner.action_index_issues_v2(
                    repo_hash="abc1234567890def",
                    project_root=tmp,
                    db_root=db_root,
                    respect_ttl=True,
                )

            self.assertTrue(result["ok"], result)
            self.assertFalse(result.get("skipped"))
            gh.assert_called()


if __name__ == "__main__":
    unittest.main()
