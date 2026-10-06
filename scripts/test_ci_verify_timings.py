"""Acceptance checks for Issue #4822 paired measurement evidence."""
import copy
import unittest

import ci_verify_timings as timings


def inventory(names):
    return {"rust-suites": {"gwt": {"testcases": {
        name: {"ignored": ignored, "filter-match": {"status": "matches"}}
        for name, ignored in names
    }}}}


def junit(pairs):
    return '<testsuites><testsuite name="gwt">' + ''.join(
        f'<testcase classname="gwt" name="{name}" time="{seconds}"/>'
        for name, seconds in pairs
    ) + '</testsuite></testsuites>'


class PairedEvidenceTests(unittest.TestCase):
    def test_same_inventory_reports_distribution_and_top_twenty_deltas(self):
        names = [(f"test_{i}", False) for i in range(25)] + [("ignored", True)]
        listed = inventory(names)
        before = junit([(name, i + 1) for i, (name, _) in enumerate(names[:-1])])
        after = junit([(name, (i + 1) / 2) for i, (name, _) in enumerate(names[:-1])])
        report = timings.compare(listed, listed, before, after)
        self.assertEqual(report["inventory"], {"total": 26, "executed": 25, "ignored": 1})
        self.assertEqual(report["before"]["distribution"]["max"], 25)
        self.assertEqual(report["after"]["distribution"]["p50"], 6.5)
        self.assertEqual(len(report["before"]["top20"]), 20)
        self.assertEqual(report["before"]["top20"][0]["name"], "test_24")
        self.assertEqual(report["deltas"][0]["delta_seconds"], -12.5)

    def test_inventory_or_execution_loss_never_becomes_success(self):
        listed = inventory([("a", False), ("b", True)])
        changed = copy.deepcopy(listed)
        changed["rust-suites"]["gwt"]["testcases"]["a"]["ignored"] = True
        with self.assertRaisesRegex(ValueError, "inventory"):
            timings.compare(listed, changed, junit([("a", 1)]), junit([]))
        for xml in [junit([]), junit([("a", "NaN")]), junit([("a", 1), ("a", 2)]),
                    '<testsuites><testsuite name="gwt"><testcase name="a" time="1"><failure/></testcase></testsuite></testsuites>']:
            with self.subTest(xml=xml), self.assertRaises(ValueError):
                timings.compare(listed, listed, junit([("a", 1)]), xml)

    def test_trace_keeps_test_identity_and_credential_child_duration(self):
        events = [
            {"event": "def_param", "sid": "git1", "param": "NEXTEST_TEST_NAME", "value": "cli::workspace::tests::a"},
            {"event": "child_start", "sid": "git1", "child_id": 0, "argv": ["git-credential-manager", "get"]},
            {"event": "child_exit", "sid": "git1", "child_id": 0, "t_rel": 2.0},
            {"event": "exit", "sid": "git1", "t_abs": 3.0},
            {"event": "atexit", "sid": "git1", "t_abs": 3.1},
        ]
        trace = timings.summarize_trace(events)["cli::workspace::tests::a"]
        self.assertEqual(trace["git_processes"], 1)
        self.assertEqual(trace["git_seconds"], 3.0)
        self.assertEqual(trace["credential_children"], 1)
        self.assertEqual(trace["credential_seconds"], 2.0)

    def test_failed_junit_identifies_test_and_measurement_phase(self):
        listed = inventory([("targeted_refresh", False)])
        passed = junit([("targeted_refresh", 1)])
        failed = passed.replace('/>', '><failure/></testcase>')
        for phase, before, after in (("before", failed, passed), ("after", passed, failed)):
            with self.subTest(phase=phase), self.assertRaisesRegex(
                    ValueError, rf"{phase}: JUnit contains failed, retried or skipped test: .*gwt.*targeted_refresh"):
                timings.compare(listed, listed, before, after)


if __name__ == "__main__":
    unittest.main()
