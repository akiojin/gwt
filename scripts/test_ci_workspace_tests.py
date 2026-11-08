"""Issue #5564: partitioning must execute the original test set exactly once."""
import json
from pathlib import Path
import tempfile
import unittest

from ci_workspace_tests import verify_shards


class WorkspaceShardTests(unittest.TestCase):
    def fixture(self, root):
        listed = {"rust-suites": {"gwt": {"testcases": {
            name: {"ignored": ignored, "filter-match": {"status": "matches"}}
            for name, ignored in [("a", False), ("b", False), ("ignored", True)]
        }}}}
        for shard, name in [(1, "a"), (2, "b")]:
            directory = root / f"workspace-tests-{shard}"
            (directory / "workspace-test-evidence").mkdir(parents=True)
            (directory / "workspace-test-evidence/inventory.json").write_text(json.dumps(listed))
            (directory / "nextest/ci").mkdir(parents=True)
            (directory / "nextest/ci/junit.xml").write_text(
                f'<testsuites><testsuite name="gwt"><testcase name="{name}" time="1"/></testsuite></testsuites>')

    def test_disjoint_results_equal_the_unpartitioned_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            self.assertEqual(verify_shards(root, 2), {
                "total": 3, "executed": 2, "ignored": 1, "per_shard": [1, 1]})

    def test_missing_shard_is_not_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with self.assertRaisesRegex(ValueError, "shard artifacts"):
                verify_shards(root, 3)

    def test_missing_duplicate_or_unexpected_execution_is_not_success(self):
        for name in ["a", "unexpected", "ignored", None]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                case = f'<testcase name="{name}" time="1"/>' if name else ""
                (root / "workspace-tests-2/nextest/ci/junit.xml").write_text(
                    f'<testsuites><testsuite name="gwt">{case}</testsuite></testsuites>')
                with self.assertRaises(ValueError):
                    verify_shards(root, 2)

    def test_shards_must_agree_on_inventory_and_ignored_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            path = root / "workspace-tests-2/workspace-test-evidence/inventory.json"
            listed = json.loads(path.read_text())
            listed["rust-suites"]["gwt"]["testcases"]["b"]["ignored"] = True
            path.write_text(json.dumps(listed))
            with self.assertRaisesRegex(ValueError, "inventory"):
                verify_shards(root, 2)


if __name__ == "__main__":
    unittest.main()
