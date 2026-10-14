"""Deterministic acceptance checks for Issue #5059's cancellation decision."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import ci_sync_relation

SCRIPT = Path(__file__).with_name("ci_sync_relation.py")


class WorkflowPolicyTests(unittest.TestCase):
    def test_paired_cancellation_waits_for_source_classification(self):
        workflow = (SCRIPT.parent.parent / ".github/workflows/test.yml").read_text()
        header, jobs = workflow.split("\njobs:", 1)
        self.assertIn("github.run_id", header,
                      "workflow-wide cancellation must not kill a base-only run")
        paired = jobs.split("\n  test-windows-verify-timings:", 1)[1].split("\n  #", 1)[0]
        self.assertIn("source-sync", paired)
        self.assertIn("cancel-in-progress: true", paired,
                      "admitted paired jobs must replace obsolete measurements")
        self.assertIn("github.event.pull_request.number || github.ref", paired)
        self.assertIn("if: ${{ !cancelled() && needs.source-sync.outputs.base_only != 'true' }}", paired,
                      "only proven base synchronization skips paired admission; unknown history still measures")
        self.assertIn("'ubuntu-latest' || 'windows-latest'", paired)
        runner = jobs.split("\n  test-python-runner:", 1)[1].split("\n  test-index-e2e:", 1)[0]
        self.assertIn("python -m unittest discover -s scripts -p 'test_ci_*.py'", runner,
                      "the source classifier regressions must run in CI")


class SyncRelationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "-b", "base")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Test")
        self.commit("source.txt", "original\n")
        self.original = self.git("rev-parse", "HEAD")
        self.git("checkout", "-b", "feature")
        self.commit("feature.txt", "feature\n")
        self.before = self.git("rev-parse", "HEAD")

    def git(self, *args, check=True):
        result = subprocess.run(
            ["git", *args], cwd=self.repo, capture_output=True, text=True, check=check
        )
        return result.stdout.strip()

    def commit(self, name, content):
        (self.repo / name).write_text(content)
        self.git("add", name)
        self.git("commit", "-m", "fixture")

    def synchronize(self):
        self.git("checkout", "base")
        self.commit("base.txt", "base advance\n")
        self.base = self.git("rev-parse", "HEAD")
        self.git("checkout", "feature")
        self.git("merge", "--no-ff", "base", "-m", "base sync")

    def relation(self, before=None):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--before", before or self.before,
             "--head", "HEAD", "--base", "base", "--repo", str(self.repo)],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_base_sync_preserves_running_jobs_and_records_non_reuse_reason(self):
        self.synchronize()
        result = self.relation()
        self.assertIs(result["base_only"], True)
        self.assertEqual(result["classification"], "bookkeeping_or_base_sync")
        self.assertEqual(result["product_commits"], [])
        self.assertIs(result["reuse_evidence"], False)
        self.assertIn("source tree", result["reuse_reason"])

    def test_actual_product_push_supersedes_running_jobs(self):
        self.commit("feature.txt", "changed\n")
        result = self.relation()
        self.assertIs(result["base_only"], False)
        self.assertEqual(result["product_commits"], [self.git("rev-parse", "HEAD")])

    def test_develop_push_starts_ci_from_its_own_commit_range(self):
        self.commit("feature.txt", "merged source\n")
        head = self.git("rev-parse", "HEAD")
        event = {"ref": "refs/heads/develop", "before": self.before, "after": head}
        result = ci_sync_relation.event_relation(self.repo, event)
        self.assertEqual(result["head"], head)
        self.assertIs(result["base_only"], False)
        self.assertEqual(result["classification"], "initial")

    def test_base_sync_cannot_hide_a_product_push_whose_classifier_was_cancelled(self):
        self.commit("feature.txt", "actual push\n")
        pushed = self.git("rev-parse", "HEAD")
        self.synchronize()
        event = {"action": "synchronize", "before": pushed, "pull_request": {
            "head": {"sha": self.git("rev-parse", "HEAD")},
            "base": {"sha": self.base},
        }}
        self.assertIs(ci_sync_relation.event_relation(self.repo, event)["base_only"], True)
        runs = [
            {"id": 1, "head_sha": self.before, "status": "in_progress",
             "created_at": "2026-10-05T17:00:00Z"},
            {"id": 2, "head_sha": pushed, "status": "completed", "conclusion": "cancelled",
             "created_at": "2026-10-05T17:01:00Z"},
            {"id": 3, "head_sha": event["pull_request"]["head"]["sha"],
             "status": "in_progress", "created_at": "2026-10-05T17:02:00Z"},
        ]
        decision = getattr(ci_sync_relation, "cancellation_decision", None)
        self.assertIsNotNone(decision, "active work must not lose an intermediate push decision")
        report = decision(self.repo, event, runs, 3)
        self.assertIs(report["base_only"], False)
        self.assertIn(pushed, report["product_commits"])
        self.assertEqual([row["run_id"] for row in report["active_run_comparisons"]], [1])
        # The Actions list may not yet include this new run.
        self.assertIs(decision(self.repo, event, runs[:2], 3)["base_only"], False)

    def test_run_history_excludes_other_prs_repositories_and_prior_branch_lifetimes(self):
        pr = {"number": 7, "created_at": "2026-10-05T17:00:00Z", "closed_at": None,
              "head": {"ref": "feature", "repo": {"id": 10}}}
        run = {"head_branch": "feature", "head_repository": {"id": 10},
               "pull_requests": [], "created_at": "2026-10-05T17:01:00Z"}
        runs = [dict(run, id=1), dict(run, id=2, pull_requests=[{"number": 7}]),
                dict(run, id=3, pull_requests=[{"number": 8}]),
                dict(run, id=4, head_repository={"id": 11}),
                dict(run, id=5, created_at="2026-10-05T16:00:00Z")]
        scope = getattr(ci_sync_relation, "runs_for_pr", None)
        self.assertIsNotNone(scope, "branch names alone must not attribute another PR's runs")
        selected, counts = scope(runs, pr)
        self.assertEqual([item["id"] for item in selected], [1, 2])
        self.assertEqual(counts, {"explicit_pr_association": 1,
                                  "inferred_head_repo_branch_lifetime": 1, "excluded": 3})

    def test_same_second_run_order_does_not_cancel_for_a_newer_base_sync(self):
        self.synchronize()
        first = self.git("rev-parse", "HEAD")
        self.git("checkout", "base")
        self.commit("base.txt", "base advances again\n")
        base = self.git("rev-parse", "HEAD")
        self.git("checkout", "feature")
        self.git("merge", "--no-ff", "base", "-m", "second base sync")
        second = self.git("rev-parse", "HEAD")
        event = {"action": "synchronize", "before": self.before, "pull_request": {
            "head": {"sha": first}, "base": {"sha": self.base},
        }}
        runs = [{"id": i, "run_number": i, "head_sha": head, "status": "in_progress",
                 "conclusion": None, "created_at": "2026-10-05T17:00:00Z",
                 "updated_at": "2026-10-05T17:01:00Z"}
                for i, head in [(1, self.before), (2, first), (3, second)]]
        decision = ci_sync_relation.cancellation_decision(self.repo, event, runs, 2)
        self.assertIs(decision["base_only"], True)
        self.assertEqual([row["run_id"] for row in decision["active_run_comparisons"]], [1])
        runs[0].update(status="completed", conclusion="cancelled")
        report = ci_sync_relation.cancellation_audit(self.repo, list(reversed(runs)), base)
        self.assertEqual(report["runs"][0]["successor_run_id"], 2)
        self.assertEqual(report["base_sync_successor_cancelled_runs"], 1)

    def test_product_commit_and_revert_are_not_hidden_by_later_base_sync(self):
        self.commit("feature.txt", "changed\n")
        changed = self.git("rev-parse", "HEAD")
        self.git("revert", "--no-edit", changed)
        self.synchronize()
        result = self.relation()
        self.assertIs(result["base_only"], False)
        self.assertIn(changed, result["product_commits"])

    def test_product_merge_resolution_is_not_base_only(self):
        self.commit("source.txt", "feature\n")
        before = self.git("rev-parse", "HEAD")
        self.git("checkout", "base")
        self.commit("source.txt", "base\n")
        self.git("checkout", "feature")
        self.git("merge", "base", check=False)
        self.commit("source.txt", "manual resolution\n")
        result = self.relation(before)
        self.assertIs(result["base_only"], False)
        self.assertIn("source.txt", result["product_files"])

    def test_missing_history_does_not_authorize_cancellation(self):
        result = self.relation("0" * 40)
        self.assertIsNone(result["base_only"])
        self.assertEqual(result["classification"], "unprovable")
        self.assertTrue(result["diagnostic"])

    def test_synchronize_event_without_before_preserves_work_and_reports_unknown(self):
        self.synchronize()
        event = {"action": "synchronize", "pull_request": {
            "head": {"sha": self.git("rev-parse", "HEAD")},
            "base": {"sha": self.base},
        }}
        event_file = self.repo / "event.json"
        event_file.write_text(json.dumps(event))
        output, summary = self.repo / "output.txt", self.repo / "summary.md"
        env = dict(os.environ, GITHUB_OUTPUT=str(output), GITHUB_STEP_SUMMARY=str(summary))
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--event-file", str(event_file),
             "--repo", str(self.repo)], env=env, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(json.loads(result.stdout)["base_only"])
        self.assertEqual(output.read_text(), "base_only=unknown\n")
        self.assertIn("cancel running jobs: `False`", summary.read_text())

    def test_cancelled_runs_are_counted_by_overlapping_successor_source_with_unknowns(self):
        self.synchronize()
        synced = self.git("rev-parse", "HEAD")
        self.commit("feature.txt", "actual push\n")
        pushed = self.git("rev-parse", "HEAD")
        runs = [
            {"id": 1, "head_sha": self.before, "conclusion": "cancelled",
             "created_at": "2026-10-05T17:00:00Z", "updated_at": "2026-10-05T17:05:00Z"},
            {"id": 2, "head_sha": synced, "conclusion": "cancelled",
             "created_at": "2026-10-05T17:04:00Z", "updated_at": "2026-10-05T17:10:00Z"},
            {"id": 3, "head_sha": pushed, "conclusion": "cancelled",
             "created_at": "2026-10-05T17:09:00Z", "updated_at": "2026-10-05T17:11:00Z"},
        ]
        audit = getattr(ci_sync_relation, "cancellation_audit", None)
        self.assertIsNotNone(audit, "AC-4 needs a cancellation-count read")
        report = audit(self.repo, runs, self.base)
        self.assertEqual(report["base_sync_successor_cancelled_runs"], 1)
        self.assertEqual(report["product_change_successor_cancelled_runs"], 1)
        self.assertEqual(report["unknown_cancelled_runs"], 1)
        self.assertEqual(report["attribution"], "inferred_from_successor_source_relation")
        # Both a preserved base sync and a later product push overlap run 1.
        runs[0]["updated_at"] = "2026-10-05T17:10:00Z"
        ambiguous = audit(self.repo, runs, self.base)
        self.assertEqual(ambiguous["base_sync_successor_cancelled_runs"], 0)
        self.assertEqual(ambiguous["unknown_cancelled_runs"], 2)


if __name__ == "__main__":
    unittest.main()
