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
                     targets="gwt|lib| gwt|test|mock", build_seconds=0,
                     list_seconds=0, run_seconds=0, budget_seconds=600):
        with tempfile.TemporaryDirectory(prefix="gwt-flake-") as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            cargo = bin_dir / "cargo"
            cargo.write_text(
                '#!/usr/bin/env bash\n'
                'printf "%s\\n" "$*" >> "$GWT_MOCK_TRACE"\n'
                'read -r clock_value < "$GWT_MOCK_CLOCK"\n'
                'case "$*" in\n'
                f'  *--no-run*) advance={build_seconds} ;;\n'
                f'  *--list*) advance={list_seconds} ;;\n'
                f'  *) advance={run_seconds} ;;\n'
                'esac\n'
                'printf "%s\\n" "$((clock_value + advance))" > "$GWT_MOCK_CLOCK"\n'
                'case "$*" in *--no-run*) exit 0 ;; esac\n'
                'case "$*" in *--list*) echo "mock: test"; exit 0 ;; esac\n'
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
            clock_value = root / "seconds"
            clock_value.write_text("0\n", encoding="utf-8", newline="\n")
            env = dict(
                os.environ,
                PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                GWT_MOCK_TRACE=trace.as_posix(),
                GWT_MOCK_CLOCK=clock_value.as_posix(),
                GWT_MOCK_FLAKY="1" if flaky else "0",
                GWT_MOCK_FAILURE_RUN=str(failure_run),
                GWT_FLAKE_TARGETS=targets,
                GWT_FLAKE_RUNS="20",
                GWT_FLAKE_BUDGET_SECS=str(budget_seconds),
                GWT_FLAKE_MIN_FREE_MB="0",
                GWT_FLAKE_LOG_DIR=(root / "logs").as_posix(),
            )
            if list_seconds:
                # An empty diff retains the same mock test on every run.
                git = bin_dir / "git"
                git.write_text("#!/usr/bin/env bash\nexit 0\n", newline="\n")
                git.chmod(0o755)
                env.update(GWT_FLAKE_BASE_SHA="HEAD", GWT_FLAKE_HEAD_SHA="HEAD")
            # Control Bash's own clock so host scheduling cannot reduce the
            # expected run count; advance it before run three for budget checks.
            clock = root / "clock.sh"
            clock_command = 'SECONDS=$(<"$GWT_MOCK_CLOCK")'
            if expire_budget:
                clock_command = (
                    "if [[ ${run:-0} == 3 ]]; then "
                    'SECONDS=$((test_started + budget_secs)); '
                    'else SECONDS=$(<"$GWT_MOCK_CLOCK"); fi'
                )
            clock.write_text(
                f"trap '{clock_command}' DEBUG\n", encoding="utf-8", newline="\n",
            )
            env["BASH_ENV"] = clock.as_posix()
            # This is a hang guard; Bash's controlled clock tests the budget.
            # Native Git Bash process startup is slower on loaded Windows hosts.
            result = subprocess.run(
                [BASH, SCRIPT.as_posix()], env=env,
                capture_output=True, text=True, timeout=300 if os.name == "nt" else 60,
            )
            runs = [line for line in trace.read_text().splitlines()
                    if "--no-run" not in line and "--list" not in line]
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
        result, _, _ = self.run_detector(flaky=True, targets="gwt|lib|")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("FLAKE: gwt|lib| differs between runs", result.stdout)

    def test_build_and_listing_reduce_optional_runs_using_the_remaining_budget(self):
        result, runs, _ = self.run_detector(
            build_seconds=80, list_seconds=10, run_seconds=5, budget_seconds=220,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sum("--lib" in run for run in runs), 2)
        integration = [run for run in runs if "--test mock" in run]
        self.assertEqual(len(integration), 6, result.stdout)
        self.assertEqual(set(integration), {
            "test -p gwt --test mock --all-features -- --exact mock",
        })
        self.assertIn("comparing 6 runs (budget share 30s)", result.stdout)

    def test_mandatory_comparison_finishes_and_reports_its_budget_overrun(self):
        result, runs, _ = self.run_detector(
            targets="gwt|lib|", build_seconds=100, run_seconds=30, budget_seconds=150,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(runs), 2)
        self.assertIn("stable: 2 run(s), every one passed", result.stdout)
        self.assertIn("exceeded its 150s budget by 10s", result.stdout)

    def test_budget_stops_new_runs_after_the_minimum_comparison(self):
        result, runs, _ = self.run_detector(expire_budget=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(runs.count("test -p gwt --test mock --all-features"), 2)
        self.assertIn("600s total detector budget reached", result.stdout)
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
    def select_suite(self, changed, merged_checkout=False):
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
            if merged_checkout:
                (crate / "tests/unrelated.rs").write_text("fn first() { assert!(true); }\n")
                for args in (["add", "."], ["commit", "--quiet", "-m", "upstream"]):
                    subprocess.run(git + args, cwd=root, check=True, capture_output=True)
                upstream = subprocess.check_output(git + ["rev-parse", "HEAD"], cwd=root, text=True).strip()
                subprocess.run(git + ["reset", "--hard", base], cwd=root, check=True, capture_output=True)
            for name in changed:
                path = crate / "tests" / name
                if changed[name] is None:
                    path.unlink()
                else:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(changed[name])
            subprocess.run(git + ["add", "."], cwd=root, check=True, capture_output=True)
            subprocess.run(git + ["commit", "--quiet", "-m", "change"], cwd=root, check=True, capture_output=True)
            head = subprocess.check_output(git + ["rev-parse", "HEAD"], cwd=root, text=True).strip()
            if merged_checkout:
                subprocess.run(git + ["merge", "--quiet", "--no-edit", upstream],
                               cwd=root, check=True, capture_output=True)
            listing = root / "tests.txt"
            listing.write_text("\n".join(f"{test}: test" for test in tests))
            result = subprocess.run(
                [sys.executable, "-c", code, "gwt|test|ci_contracts", str(listing), base],
                cwd=root, env=dict(os.environ, PYTHONPATH=str(SCRIPT.parent.resolve()),
                                  GWT_FLAKE_HEAD_SHA=head),
                capture_output=True, text=True, check=True,
            )
            return result.stdout.splitlines()

    def test_consolidated_suite_repeats_only_the_changed_file(self):
        self.assertEqual(self.select_suite({"changed.rs": "fn first() { assert!(true); }\n"}),
                         ["changed::first", "changed::first_extra"])

    def test_merge_checkout_excludes_upstream_only_changes(self):
        self.assertEqual(self.select_suite({"changed.rs": "fn first() { assert!(true); }\n"},
                                           merged_checkout=True),
                         ["changed::first", "changed::first_extra"])

    def test_suite_harness_or_unknown_or_deleted_source_retains_the_full_target(self):
        for changed in (
            {"suites/ci_contracts.rs": "// changed wrapper\n"},
            {"changed.rs": "fn first() {} // changed\n", "helper.rs": "fn helper() {}\n"},
            {"changed.rs": "fn first() {} // changed\n", "../src/lib.rs": "fn changed() {}\n"},
            {"changed.rs": None},
        ):
            with self.subTest(changed=changed):
                self.assertEqual(self.select_suite(changed),
                                 ["changed::first", "changed::first_extra", "unrelated::first"])


if __name__ == "__main__":
    unittest.main()
