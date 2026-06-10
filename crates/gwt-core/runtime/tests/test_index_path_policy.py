"""Tests for the shared project-index path policy."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import chroma_index_runner as runner


class IndexPathPolicyTests(unittest.TestCase):
    def test_collect_files_honors_nested_gitignore(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            app = root / "packages" / "app"
            app.mkdir(parents=True)
            (app / ".gitignore").write_text("*.generated\n", encoding="utf-8")
            (app / "view.generated").write_text("ignored\n", encoding="utf-8")
            (app / "view.rs").write_text("fn view() {}\n", encoding="utf-8")

            rels = {
                path.relative_to(root).as_posix()
                for path in runner.collect_files(root)
            }

            self.assertIn("packages/app/view.rs", rels)
            self.assertNotIn("packages/app/view.generated", rels)

    def test_collect_files_honors_git_info_exclude(self):
        if shutil.which("git") is None:
            self.skipTest("git is required for info/exclude policy coverage")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            subprocess.run(["git", "init", str(root)], check=True, capture_output=True)
            (root / ".git" / "info" / "exclude").write_text(
                "local-secret.txt\n",
                encoding="utf-8",
            )
            (root / "local-secret.txt").write_text("secret\n", encoding="utf-8")
            (root / "src.rs").write_text("fn main() {}\n", encoding="utf-8")

            rels = {
                path.relative_to(root).as_posix()
                for path in runner.collect_files(root)
            }

            self.assertIn("src.rs", rels)
            self.assertNotIn("local-secret.txt", rels)

    def test_collect_files_denies_common_generated_directories(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            for rel in [
                ".venv/lib/python/site.py",
                ".pytest_cache/v/cache/nodeids",
                ".gradle/caches/modules-2.bin",
                ".terraform/providers/state.json",
                "coverage/lcov.info",
            ]:
                path = root / rel
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("generated\n", encoding="utf-8")
            (root / "src.rs").write_text("fn main() {}\n", encoding="utf-8")

            rels = {
                path.relative_to(root).as_posix()
                for path in runner.collect_files(root)
            }

            self.assertEqual(rels, {"src.rs"})

    def test_shared_policy_exposes_memory_allowlist(self):
        policy = runner.load_index_path_policy()

        self.assertIn(".gwt/work/memory.md", policy["allow_paths"])
        self.assertIn(".gwt/work/discussions.md", policy["allow_paths"])
        self.assertIn(".gwt", policy["deny_root_prefixes"])


if __name__ == "__main__":
    unittest.main()
