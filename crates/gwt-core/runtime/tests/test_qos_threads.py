"""Phase 70 T-IDX-385 (Issue #3264): runner QoS thread / priority contract.

FR-385: thread environment must be configured before any ML / numeric
library import. Background QoS caps embedding threads at 2 and lowers the
process priority (Unix nice 10 / Windows Below Normal); interactive QoS caps
at 4. Inter-op parallelism is 1 and tokenizer parallelism is disabled.
"""

from __future__ import annotations

import os
import sys
import types
import unittest
from unittest import mock

import chroma_index_runner as runner

THREAD_ENV_KEYS = (
    "OMP_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS",
    "NUMEXPR_NUM_THREADS",
)


class QosThreadEnvTests(unittest.TestCase):
    def setUp(self):
        self._saved = {
            key: os.environ.get(key)
            for key in (*THREAD_ENV_KEYS, "TOKENIZERS_PARALLELISM")
        }
        for key in self._saved:
            os.environ.pop(key, None)

    def tearDown(self):
        for key, value in self._saved.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    def test_background_qos_caps_embedding_threads_at_two(self):
        runner.configure_qos_threads("background")
        for key in THREAD_ENV_KEYS:
            self.assertEqual(
                os.environ.get(key),
                "2",
                f"background QoS must cap {key} at 2 (FR-385)",
            )
        self.assertEqual(os.environ.get("TOKENIZERS_PARALLELISM"), "false")

    def test_interactive_qos_caps_embedding_threads_at_four(self):
        runner.configure_qos_threads("interactive")
        for key in THREAD_ENV_KEYS:
            self.assertEqual(
                os.environ.get(key),
                "4",
                f"interactive QoS must cap {key} at 4 (FR-385)",
            )
        self.assertEqual(os.environ.get("TOKENIZERS_PARALLELISM"), "false")

    def test_qos_configuration_does_not_import_heavy_modules(self):
        # The QoS path must stay torch/chroma-free so the environment is in
        # place before the lazy sentence-transformers import (FR-385) and so
        # light actions never pay the model import.
        for module in ("torch", "sentence_transformers"):
            self.assertNotIn(module, sys.modules)
        runner.configure_qos_threads("background")
        for module in ("torch", "sentence_transformers"):
            self.assertNotIn(
                module,
                sys.modules,
                f"configure_qos_threads must not import {module}",
            )

    def test_qos_applies_torch_threads_when_torch_already_loaded(self):
        fake_torch = types.ModuleType("torch")
        fake_torch.set_num_threads = mock.Mock()
        fake_torch.set_num_interop_threads = mock.Mock()
        with mock.patch.dict(sys.modules, {"torch": fake_torch}):
            runner.configure_qos_threads("interactive")
            fake_torch.set_num_threads.assert_called_once_with(4)
            fake_torch.set_num_interop_threads.assert_called_once_with(1)

    @unittest.skipIf(os.name == "nt", "POSIX process priority uses nice")
    def test_background_qos_lowers_process_priority_on_posix(self):
        with mock.patch("os.nice") as fake_nice:
            runner.configure_qos_threads("background")
        fake_nice.assert_called_once_with(10)

    @unittest.skipIf(os.name == "nt", "POSIX process priority uses nice")
    def test_interactive_qos_keeps_normal_priority(self):
        with mock.patch("os.nice") as fake_nice:
            runner.configure_qos_threads("interactive")
        fake_nice.assert_not_called()


class QosArgumentTests(unittest.TestCase):
    def _parse(self, argv):
        with mock.patch.object(sys, "argv", ["chroma_index_runner.py", *argv]):
            return runner.parse_args()

    def test_parse_args_accepts_qos_background(self):
        args = self._parse(
            [
                "--action",
                "index-files",
                "--repo-hash",
                "a" * 16,
                "--worktree-hash",
                "b" * 16,
                "--project-root",
                ".",
                "--qos",
                "background",
            ]
        )
        self.assertEqual(args.qos, "background")

    def test_parse_args_accepts_qos_interactive(self):
        args = self._parse(
            [
                "--action",
                "search-multi",
                "--repo-hash",
                "a" * 16,
                "--scopes",
                "issues,specs",
                "--query",
                "q",
                "--qos",
                "interactive",
            ]
        )
        self.assertEqual(args.qos, "interactive")

    def test_default_qos_derives_from_action(self):
        # Index builds are background work; searches and light actions are
        # interactive by default so legacy callers keep working (FR-398).
        self.assertEqual(runner.default_qos_for_action("index-files"), "background")
        self.assertEqual(runner.default_qos_for_action("index-issues"), "background")
        self.assertEqual(runner.default_qos_for_action("search-multi"), "interactive")
        self.assertEqual(runner.default_qos_for_action("search-files"), "interactive")
        self.assertEqual(runner.default_qos_for_action("status"), "interactive")


if __name__ == "__main__":
    unittest.main()
