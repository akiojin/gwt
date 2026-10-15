"""Issue #5171 acceptance checks against the actual Bash detector."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("ci-flake-detect.sh")
BASH = shutil.which("bash") if os.name != "nt" else r"C:/Program Files/Git/bin/bash.exe"


class FlakeDetectionTests(unittest.TestCase):
    def run_detector(self, flaky=False, expire_budget=False, failure_run=2,
                     targets="gwt|lib| gwt|test|mock"):
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
                GWT_FLAKE_TARGETS=targets,
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
                # This case checks the two library logs; twenty unrelated
                # integration repetitions add process startup cost on Windows.
                result, _, logs = self.run_detector(
                    flaky=True, failure_run=failure_run, targets="gwt|lib|",
                )
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                for detail in ("panicked at mock.rs:12:5", "assertion `left == right` failed"):
                    self.assertIn(detail, result.stdout)
                    self.assertIn(detail, logs[f"gwt_lib_-run-{failure_run}.log"])
                self.assertIn("gwt_lib_-run-1.log", logs)
                self.assertIn("gwt_lib_-run-2.log", logs)


class ChangedTestSelectionTests(unittest.TestCase):
    def select_suite(self, changed):
        code = SCRIPT.read_text().split("# changed-test-selection-begin\n", 1)[1].split(
            "# changed-test-selection-end\n", 1
        )[0]
        tests = ["changed::first", "changed::first_extra", "unrelated::first"]
        with tempfile.TemporaryDirectory(prefix="gwt-flake-selection-") as directory:
            root = Path(directory)
            crate = root / "crates/gwt"
            (crate / "tests/suites").mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[[test]]\nname = "ci_contracts"\npath = "tests/suites/ci_contracts.rs"\n'
            )
            (crate / "tests/suites/ci_contracts.rs").write_text(
                '#[path = "../changed.rs"]\nmod changed;\n'
                '#[path = "../unrelated.rs"]\nmod unrelated;\n'
            )
            (crate / "tests/changed.rs").write_text("fn first() {}\n")
            (crate / "tests/unrelated.rs").write_text("fn first() {}\n")
            git = ["git", "-c", "user.name=CI fixture", "-c", "user.email=ci@example.invalid"]
            for args in (["init", "--quiet"], ["add", "."], ["commit", "--quiet", "-m", "base"]):
                subprocess.run(git + args, cwd=root, check=True, capture_output=True)
            base = subprocess.check_output(git + ["rev-parse", "HEAD"], cwd=root, text=True).strip()
            for name in changed:
                path = crate / "tests" / name
                if changed[name] is None:
                    path.unlink()
                else:
                    path.write_text(changed[name])
            subprocess.run(git + ["add", "."], cwd=root, check=True, capture_output=True)
            subprocess.run(git + ["commit", "--quiet", "-m", "change"], cwd=root, check=True, capture_output=True)
            listing = root / "tests.txt"
            listing.write_text("\n".join(f"{test}: test" for test in tests))
            result = subprocess.run(
                [sys.executable, "-c", code, "gwt|test|ci_contracts", str(listing), base],
                cwd=root, env=dict(os.environ, PYTHONPATH=str(SCRIPT.parent.resolve())),
                capture_output=True, text=True, check=True,
            )
            return result.stdout.splitlines()

    def test_consolidated_suite_repeats_only_the_changed_file(self):
        self.assertEqual(self.select_suite({"changed.rs": "fn first() { assert!(true); }\n"}),
                         ["changed::first", "changed::first_extra"])

    def test_suite_harness_or_unknown_or_deleted_source_retains_the_full_target(self):
        for changed in (
            {"suites/ci_contracts.rs": "// changed wrapper\n"},
            {"changed.rs": "fn first() {} // changed\n", "helper.rs": "fn helper() {}\n"},
            {"changed.rs": None},
        ):
            with self.subTest(changed=changed):
                self.assertEqual(self.select_suite(changed),
                                 ["changed::first", "changed::first_extra", "unrelated::first"])


if __name__ == "__main__":
    unittest.main()
