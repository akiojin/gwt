"""Issue #5171 acceptance checks against the actual Bash detector."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("ci-flake-detect.sh")
BASH = shutil.which("bash") if os.name != "nt" else r"C:/Program Files/Git/bin/bash.exe"


class FlakeDetectionTests(unittest.TestCase):
    def run_detector(self, flaky=False, expire_budget=False, failure_run=2):
        with tempfile.TemporaryDirectory(prefix="gwt-flake-") as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            cargo = bin_dir / "cargo"
            cargo.write_text(
                '#!/usr/bin/env bash\n'
                'printf "%s\\n" "$*" >> "$GWT_MOCK_TRACE"\n'
                'case "$*" in *--no-run*) exit 0 ;; esac\n'
                'if [ "$GWT_MOCK_FLAKY" = 1 ] && '
                '[ "$(grep -Fxc "$*" "$GWT_MOCK_TRACE")" = "$GWT_MOCK_FAILURE_RUN" ]; then\n'
                "  cat <<'FAILURE'\n"
                'test mock ... FAILED\n'
                'failures:\n'
                '---- mock stdout ----\n'
                "thread 'mock' panicked at mock.rs:12:5:\n"
                "assertion `left == right` failed\n"
                '  left: 1\n'
                ' right: 2\n'
                'FAILURE\n'
                '  exit 101\n'
                'fi\n'
                'echo "test mock ... ok"\n',
                encoding="utf-8",
                newline="\n",
            )
            cargo.chmod(0o755)
            trace = root / "trace"
            env = dict(
                os.environ,
                PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                GWT_MOCK_TRACE=trace.as_posix(),
                GWT_MOCK_FLAKY="1" if flaky else "0",
                GWT_MOCK_FAILURE_RUN=str(failure_run),
                GWT_FLAKE_TARGETS="gwt|lib| gwt|test|mock",
                GWT_FLAKE_RUNS="20",
                GWT_FLAKE_BUDGET_SECS="600",
                GWT_FLAKE_MIN_FREE_MB="0",
                GWT_FLAKE_LOG_DIR=(root / "logs").as_posix(),
            )
            # Control Bash's own clock so host scheduling cannot reduce the
            # expected run count; advance it before run three for budget checks.
            clock = root / "clock.sh"
            clock_command = "SECONDS=0"
            if expire_budget:
                clock_command = (
                    "if [[ ${run:-0} == 3 ]]; then "
                    "SECONDS=$((test_started + budget_secs)); else SECONDS=0; fi"
                )
            clock.write_text(
                f"trap '{clock_command}' DEBUG\n", encoding="utf-8", newline="\n",
            )
            env["BASH_ENV"] = clock.as_posix()
            result = subprocess.run(
                [BASH, SCRIPT.as_posix()], env=env,
                capture_output=True, text=True, timeout=60,
            )
            runs = [line for line in trace.read_text().splitlines() if "--no-run" not in line]
            logs = {path.name: path.read_text() for path in (root / "logs").glob("*.log")}
            return result, runs, logs

    def test_lib_stops_at_two_while_integration_can_run_twenty(self):
        result, runs, _ = self.run_detector()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(runs.count("test -p gwt --lib --all-features"), 2)
        self.assertEqual(runs.count("test -p gwt --test mock --all-features"), 20)
        self.assertIn("stable: 2 run(s), every one passed", result.stdout)
        self.assertIn("stable: 20 run(s), every one passed", result.stdout)

    def test_changed_outcome_still_fails_with_the_shorter_lib_run_count(self):
        result, _, _ = self.run_detector(flaky=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("FLAKE: gwt|lib| differs between runs", result.stdout)

    def test_budget_stops_new_runs_after_the_minimum_comparison(self):
        result, runs, _ = self.run_detector(expire_budget=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(runs.count("test -p gwt --test mock --all-features"), 2)
        self.assertIn("600s test execution budget reached", result.stdout)
        self.assertIn("incomplete check", result.stdout)

    def test_flake_reports_and_retains_panic_from_either_run(self):
        for failure_run in (1, 2):
            with self.subTest(failure_run=failure_run):
                result, _, logs = self.run_detector(flaky=True, failure_run=failure_run)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                for detail in ("panicked at mock.rs:12:5", "assertion `left == right` failed"):
                    self.assertIn(detail, result.stdout)
                    self.assertIn(detail, logs[f"gwt_lib_-run-{failure_run}.log"])
                self.assertIn("gwt_lib_-run-1.log", logs)
                self.assertIn("gwt_lib_-run-2.log", logs)


if __name__ == "__main__":
    unittest.main()
