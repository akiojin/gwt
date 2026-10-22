import tempfile
import unittest
from pathlib import Path

import test_layout


class TestLayout(unittest.TestCase):
    def fixture(self, root):
        crate = root / "crates/gwt"
        (crate / "tests/suites").mkdir(parents=True)
        (crate / "src/app_runtime").mkdir(parents=True)
        (crate / "Cargo.toml").write_text(
            '[package]\nautotests = false\n[[test]]\nname = "cli_contracts"\npath = "tests/suites/cli_contracts_test.rs"\n',
            encoding="utf-8",
        )
        (crate / "tests/suites/cli_contracts_test.rs").write_text(
            '#[path = "../gwtd_cli_test.rs"]\nmod gwtd_cli_test;\n', encoding="utf-8"
        )
        (crate / "tests/gwtd_cli_test.rs").write_text("", encoding="utf-8")
        (crate / "src/app_runtime/tests.rs").write_text("mod startup_tests;\n", encoding="utf-8")

    def suite(self, binary, kind, name, ignored=False):
        return {"package-name": "gwt", "kind": kind, "binary-name": binary,
                "testcases": {name: {"ignored": ignored}}}

    def test_compiled_names_survive_both_layout_changes_and_reject_loss_or_ignored_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            before = {"rust-suites": {
                "cli": self.suite("gwtd_cli_test", "test", "help"),
                "runtime": self.suite("gwt", "bin", "app_runtime::tests::restore", True),
            }}
            after = {"rust-suites": {
                "cli": self.suite("cli_contracts", "test", "gwtd_cli_test::help"),
                "runtime": self.suite("gwt", "bin", "app_runtime::tests::startup_tests::restore", True),
            }}
            previous = test_layout.inventory_tests(before, root)
            current = test_layout.inventory_tests(after, root)
            self.assertEqual(previous, current)
            name = next(iter(current))
            current[name] = not current[name]
            self.assertEqual(test_layout.compare(previous, current)["ignored_changed"], [name])
            del current[name]
            self.assertEqual(test_layout.compare(previous, current)["missing"], [name])

    def test_changed_original_and_wrapper_resolve_to_one_registered_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            self.assertEqual(test_layout.changed_targets([
                "crates/gwt/tests/gwtd_cli_test.rs",
                "crates/gwt/tests/suites/cli_contracts_test.rs",
                "crates/gwt/tests/fixtures/example.rs",
            ], root), ["gwt|test|cli_contracts"])


if __name__ == "__main__":
    unittest.main()
