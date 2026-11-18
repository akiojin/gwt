"""Issue #7423: standalone consumers retain Cargo's native search paths."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


fixture = Path(__file__).resolve().parents[1] / "crates/gwt-core/tests/fixtures/run_windows_cmd_runner_probe.py"
spec = importlib.util.spec_from_file_location("cmd_probe", fixture)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class CmdProbeTests(unittest.TestCase):
    def test_direct_rustc_retains_build_script_native_search_paths(self):
        native = r"native=C:\cargo registry\windows_x86_64_msvc-0.52.6\lib"
        messages = [{"reason": "build-script-executed", "linked_paths": [native]}]
        messages += [{"reason": "compiler-artifact", "target": {"name": name},
                      "filenames": [f"target/debug/deps/lib{name}.rlib"]}
                     for name in ("gwt_core", "gwt_agent", "gwt_terminal")]
        commands = []

        def run(command, root):
            commands.append(command)
            stdout = "\n".join(map(json.dumps, messages)) if command[0] == "cargo" else ""
            return subprocess.CompletedProcess(command, 0, stdout, "")

        with tempfile.TemporaryDirectory() as target, \
                patch.dict(os.environ, {"CARGO_TARGET_DIR": target}), \
                patch.object(sys, "argv", [str(fixture), "--compile-only"]), \
                patch.object(probe, "run", side_effect=run), \
                patch.object(probe, "sha256", return_value="fixture-hash"):
            self.assertEqual(probe.main(), 0)
        compile_command = next(command for command in commands if "--edition=2021" in command)
        search_paths = [compile_command[index + 1] for index, arg in enumerate(compile_command) if arg == "-L"]
        self.assertIn(native, search_paths)


if __name__ == "__main__":
    unittest.main()
