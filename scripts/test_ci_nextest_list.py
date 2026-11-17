"""The CI listing deadline must fail the command and reap the listed binary."""

import os
from pathlib import Path
import re
import subprocess
import sys
import unittest


WRAPPER = Path(__file__).with_name("ci_nextest_list.py")


@unittest.skipUnless(os.name == "posix", "the CI wrapper uses Unix process groups")
class NextestListTests(unittest.TestCase):
    def run_wrapper(self, deadline, code):
        return subprocess.run(
            [
                sys.executable,
                str(WRAPPER),
                deadline,
                sys.executable,
                "-c",
                code,
                "--list",
                "--format",
                "terse",
                "--ignored",
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_listing_preserves_arguments_output_and_exit_status(self):
        for code in (0, 17):
            with self.subTest(exit_code=code):
                result = self.run_wrapper(
                    "5",
                    "import sys; print(' '.join(sys.argv[1:])); "
                    f"print('diagnostic', file=sys.stderr); sys.exit({code})",
                )
                self.assertEqual(result.returncode, code, result.stderr)
                self.assertEqual(result.stdout, "--list --format terse --ignored\n")
                self.assertEqual(result.stderr, "diagnostic\n")

    def test_stalled_listing_reports_and_reaps_the_process(self):
        result = self.run_wrapper("0.5", "import signal; signal.pause()")
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertIn("--list --format terse --ignored", result.stderr)
        pid = re.search(r"timed out after 0\.5s \(pid (\d+)\)", result.stderr)
        self.assertIsNotNone(pid, result.stderr)
        self.assertRegex(result.stderr, rf"(?m)^\s*{pid[1]}\s+\d+\s+\S+")
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid[1]), 0)


if __name__ == "__main__":
    unittest.main()
