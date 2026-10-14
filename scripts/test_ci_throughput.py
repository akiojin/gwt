"""Acceptance checks for Issue #5174's reproducible CI measurements."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import ci_throughput as throughput


def sample():
    return {"schema_version": 1, "repo": "owner/repo", "base": "develop",
            "before": "2026-10-08T00:30:00Z", "limit": 2,
            "pull_requests": [
                {"number": number, "created_at": "2026-10-07T20:00:00Z",
                 "merged_at": merged, "sync_commits": syncs,
                 "runs": [{"id": number, "run_attempt": 1, "status": "completed",
                           "conclusion": "success", "created_at": "2026-10-07T21:00:00Z",
                           "run_started_at": "2026-10-07T21:00:00Z",
                           "updated_at": ended,
                           "jobs": [{"id": number, "name": "Test (Rust)",
                                     "created_at": "2026-10-07T21:00:00Z",
                                     "started_at": "2026-10-07T21:02:00Z",
                                     "completed_at": ended, "conclusion": "success"},
                                    {"id": number + 10, "name": "Skipped job",
                                     "created_at": "2026-10-07T21:00:00Z",
                                     "started_at": None, "completed_at": None,
                                     "conclusion": "skipped"}]}]}
                for number, merged, ended, syncs in (
                    (1, "2026-10-07T22:00:00Z", "2026-10-07T21:30:00Z", ["a"]),
                    (2, "2026-10-08T00:00:00Z", "2026-10-07T21:40:00Z", ["b", "c"]))]}


class ThroughputTests(unittest.TestCase):
    def test_all_requested_metrics_have_explicit_units_and_sample_counts(self):
        report = throughput.summarize(sample())
        self.assertEqual(report["pr_created_to_merged_minutes"]["p50"], 180)
        self.assertEqual(report["pr_created_to_merged_minutes"]["p90"], 240)
        self.assertEqual(report["workflow_cycle_minutes"]["p50"], 35)
        self.assertEqual(report["base_syncs_per_merge"], 1.5)
        self.assertEqual(report["runner_wait_minutes"]["p50"], 2)
        self.assertEqual(report["jobs"]["Test (Rust)"]["duration_minutes"]["p50"], 33)
        self.assertEqual(report["jobs"]["Test (Rust)"]["duration_minutes"]["p90"], 38)
        self.assertNotIn("Skipped job", report["jobs"])
        self.assertEqual(report["workflow_cycle_minutes"]["count"], 2)

    def test_cancelled_runs_do_not_become_complete_cycle_samples(self):
        data = sample()
        cancelled = copy.deepcopy(data["pull_requests"][0]["runs"][0])
        cancelled.update(id=999, conclusion="cancelled", updated_at="2026-10-07T23:00:00Z")
        data["pull_requests"][0]["runs"].append(cancelled)
        report = throughput.summarize(data)
        self.assertEqual(report["workflow_cycle_minutes"]["p50"], 35)
        self.assertEqual(report["excluded_runs"]["cancelled"], 1)

    def test_missing_measurements_are_unavailable_instead_of_zero(self):
        data = sample()
        data["pull_requests"][0]["sync_commits"] = None
        data["pull_requests"][1]["runs"] = []
        data["pull_requests"][0]["runs"][0]["jobs"][0].pop("created_at")
        report = throughput.summarize(data)
        self.assertIsNone(report["base_syncs_per_merge"])
        self.assertIsNone(report["runner_wait_minutes"]["p50"])
        self.assertEqual(report["workflow_cycle_minutes"]["count"], 1)
        self.assertEqual(report["missing_workflow_prs"], [2])

    def test_rerun_reused_job_does_not_report_negative_or_zero_runner_wait(self):
        data = sample()
        job = data["pull_requests"][0]["runs"][0]["jobs"][0]
        job["created_at"] = "2026-10-07T22:00:00Z"
        report = throughput.summarize(data)
        self.assertEqual(report["runner_wait_minutes"]["count"], 1)
        self.assertEqual(report["unavailable_runner_wait_jobs"], [1])
        self.assertEqual(report["jobs"]["Test (Rust)"]["duration_minutes"]["count"], 2)

    def test_replay_cli_uses_the_same_calculation_without_git_or_github(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "snapshot.json"
            path.write_text(json.dumps(sample()), encoding="utf-8")
            result = subprocess.run([sys.executable, str(Path(throughput.__file__)),
                                     "--input", str(path)], capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(result.stdout), throughput.summarize(sample()))

    def test_collector_selects_merged_order_not_updated_order_and_paginates(self):
        newer = {"number": 2, "merged_at": "2026-10-07T22:00:00Z",
                 "updated_at": "2026-10-07T23:00:00Z"}
        older = {"number": 1, "merged_at": "2026-10-07T21:00:00Z",
                 "updated_at": "2026-10-08T02:00:00Z"}
        future = {"number": 3, "merged_at": "2026-10-08T01:00:00Z",
                  "updated_at": "2026-10-08T01:00:00Z"}
        with patch.object(throughput, "api", side_effect=[[older, future], [newer], []]):
            selected = throughput.merged_prs("owner/repo", "develop", 1,
                                            "2026-10-08T00:30:00Z", page_size=2)
        self.assertEqual([pr["number"] for pr in selected], [2])

    def test_baseline_snapshot_reproduces_published_rounded_values(self):
        baseline = Path(__file__).with_name("fixtures") / "ci-throughput-2026-10-08.json"
        report = throughput.summarize(json.loads(baseline.read_text(encoding="utf-8")))
        self.assertEqual(report["pr_count"], 25)
        self.assertEqual(round(report["pr_created_to_merged_minutes"]["p50"]), 130)
        self.assertEqual(round(report["workflow_cycle_minutes"]["p50"]), 32)
        self.assertAlmostEqual(report["base_syncs_per_merge"], 4.6)
        self.assertAlmostEqual(report["historical_message_filter_syncs_per_merge"], 4.4)

    def test_base_sync_counts_custom_subjects_and_excludes_side_branch_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            throughput.git(root, "init", "--quiet")
            throughput.git(root, "config", "user.name", "Fixture")
            throughput.git(root, "config", "user.email", "fixture@example.invalid")
            tree = subprocess.check_output(["git", "mktree"], cwd=root, input="", text=True).strip()

            def commit(subject, *parents):
                args = [value for parent in parents for value in ("-p", parent)]
                return throughput.git(root, "commit-tree", tree, *args, "-m", subject)

            base = commit("base")
            feature = commit("feature", base)
            other = commit("other feature", base)
            advanced = commit("next base", base)
            imported = commit("Merge side branch", feature, other)
            custom_sync = commit("chore: update base", imported, advanced)
            final_base = commit("Merge other feature", advanced, other)
            evidence = throughput.base_sync_commits(root, final_base, custom_sync)
        self.assertEqual([row["sha"] for row in evidence if row["is_base_sync"]], [custom_sync])


if __name__ == "__main__":
    unittest.main()
