"""Regression tests for the project-root hash fallback (Issue #2933).

When an agent pane is launched without GWT_REPO_HASH / GWT_WORKTREE_HASH in
its environment, skill-driven search commands pass only --project-root. The
runner must then derive the repo/worktree hashes itself so the v2 search
pipeline engages instead of failing with "--db-path is required".

The derived hashes MUST match the Rust canonical implementations
(gwt_core::repo_hash::compute_repo_hash + gwt_core::worktree_hash::
compute_worktree_hash) byte-for-byte, because the on-disk index directory is
named by the Rust-computed hash.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class NormalizeOriginUrlTests(unittest.TestCase):
    def test_all_forms_normalize_to_canonical_host_path(self):
        cases = [
            "https://github.com/akiojin/gwt",
            "https://github.com/akiojin/gwt.git",
            "https://github.com/Akiojin/GWT.git",
            "https://github.com/akiojin/gwt/",
            "git@github.com:akiojin/gwt.git",
            "ssh://git@github.com:22/akiojin/gwt.git",
            "https://user@github.com:443/akiojin/gwt.git",
            "github.com/akiojin/gwt",
        ]
        for url in cases:
            self.assertEqual(
                runner.normalize_origin_url(url),
                "github.com/akiojin/gwt",
                msg=f"normalize failed for {url!r}",
            )

    def test_compute_repo_hash_matches_rust_production_value(self):
        # sha256("github.com/akiojin/gwt")[:16] == the live index dir name.
        self.assertEqual(
            runner.compute_repo_hash("https://github.com/akiojin/gwt"),
            "99a8660247f5bc49",
        )

    def test_https_and_ssh_yield_same_hash(self):
        self.assertEqual(
            runner.compute_repo_hash("https://github.com/akiojin/gwt.git"),
            runner.compute_repo_hash("git@github.com:akiojin/gwt.git"),
        )


class DeriveHashesFromProjectRootTests(unittest.TestCase):
    def test_derives_repo_and_worktree_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            url = "https://github.com/akiojin/gwt.git"
            with mock.patch.object(runner, "_git_origin_url", return_value=url):
                derived = runner._derive_hashes_from_project_root(str(root))

            self.assertIsNotNone(derived)
            self.assertEqual(derived["repo_hash"], runner.compute_repo_hash(url))
            expected_wt = hashlib.sha256(
                str(root.resolve()).encode("utf-8")
            ).hexdigest()[:16]
            self.assertEqual(derived["worktree_hash"], expected_wt)

    def test_returns_none_when_no_origin(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.object(runner, "_git_origin_url", return_value=None):
                self.assertIsNone(
                    runner._derive_hashes_from_project_root(tmp)
                )

    def test_returns_none_without_project_root(self):
        self.assertIsNone(runner._derive_hashes_from_project_root(""))


class MainProjectRootFallbackTests(unittest.TestCase):
    """End-to-end: reproduce the agent-pane scenario (no hashes in env)."""

    def _make_repo(self, root: Path) -> None:
        (root / "src").mkdir(parents=True)
        (root / "src" / "watcher.rs").write_text(
            "//! file system watcher with debounce\nfn debounce_events() {}\n"
        )
        (root / "README.md").write_text("# project\n")

    def test_search_files_without_repo_hash_derives_from_project_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            self._make_repo(root)

            argv = [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--project-root",
                str(root),
                "--query",
                "watcher",
            ]
            buf = io.StringIO()
            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False), \
                mock.patch.object(
                    runner,
                    "_git_origin_url",
                    return_value="https://github.com/example/proj.git",
                ), \
                mock.patch.object(sys, "argv", argv), \
                redirect_stdout(buf):
                rc = runner.main()

            self.assertEqual(rc, 0, msg=buf.getvalue())
            payload = json.loads(buf.getvalue().strip().splitlines()[-1])
            self.assertTrue(payload.get("ok"), msg=buf.getvalue())
            paths = [item["path"] for item in payload.get("results", [])]
            self.assertTrue(
                any("watcher" in p for p in paths),
                msg=f"expected watcher.rs in results, got {paths}",
            )


if __name__ == "__main__":
    unittest.main()
