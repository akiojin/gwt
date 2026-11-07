"""Issue #6809: Rust suites must not add stores to the runner's real HOME."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from ci_home_guard import check_snapshot, write_snapshot


class HomeGuardTests(unittest.TestCase):
    def test_unchanged_or_removed_stores_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            projects = root / "home/.gwt/projects"
            projects.mkdir(parents=True)
            (projects / "existing").mkdir()
            state = root / "before.json"
            with contextlib.redirect_stdout(io.StringIO()):
                write_snapshot(state, projects)
                self.assertEqual(check_snapshot(state), 0)
                (projects / "existing").rmdir()
                self.assertEqual(check_snapshot(state), 0)

    def test_new_store_is_rejected_even_when_entry_count_is_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            projects = root / "home/.gwt/projects"
            projects.mkdir(parents=True)
            (projects / "old").mkdir()
            state = root / "before.json"
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                write_snapshot(state, projects)
                (projects / "old").rmdir()
                (projects / "leaked").mkdir()
                self.assertEqual(check_snapshot(state), 1)
            self.assertIn('"added": ["leaked"]', output.getvalue())
            self.assertTrue((projects / "leaked").is_dir(), "diagnostics never clean up evidence")

    def test_absent_real_home_is_not_created_by_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            projects = root / "home/.gwt/projects"
            state = root / "state/before.json"
            with contextlib.redirect_stdout(io.StringIO()):
                write_snapshot(state, projects)
                self.assertEqual(check_snapshot(state), 0)
            self.assertFalse(projects.parent.exists())
            self.assertEqual(json.loads(state.read_text())["entries"], [])


if __name__ == "__main__":
    unittest.main()
