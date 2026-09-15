"""Phase 8: tests for --repo-hash / --worktree-hash / --scope path resolution.

These tests will fail until the runner is redesigned to compute db_path internally
from (repo_hash, worktree_hash, scope) instead of accepting --db-path directly.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class ResolveDbPathTests(unittest.TestCase):
    def test_file_index_v2_root_is_additive_and_repo_scoped(self):
        resolver = getattr(runner, "resolve_file_index_v2_root", None)
        self.assertIsNotNone(
            resolver,
            "Phase 71 requires the additive resolve_file_index_v2_root API",
        )

        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index"
            actual = resolver("abc1234567890def", db_root=db_root)
            self.assertEqual(
                actual,
                db_root / "abc1234567890def" / "file-index-v2",
            )

    def test_issue_scope_omits_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="issues",
        )
        expected = (".gwt", "index", "abc1234567890def", "issues")
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_specs_scope_is_repo_scoped(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="specs",
        )
        expected = (
            ".gwt",
            "index",
            "abc1234567890def",
            "specs",
        )
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_files_scope_includes_worktree_hash(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="files",
        )
        expected = (
            ".gwt",
            "index",
            "abc1234567890def",
            "worktrees",
            "111122223333ffff",
            "files",
        )
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_files_docs_scope(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="files-docs",
        )
        expected = (
            ".gwt",
            "index",
            "abc1234567890def",
            "worktrees",
            "111122223333ffff",
            "files-docs",
        )
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_memory_scope_is_repo_scoped(self):
        path = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="memory",
        )
        expected = (".gwt", "index", "abc1234567890def", "memory")
        self.assertEqual(path.parts[-len(expected) :], expected)

    def test_memory_scope_ignores_worktree_hash(self):
        with_wt = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash="111122223333ffff",
            scope="memory",
        )
        without_wt = runner.resolve_db_path(
            repo_hash="abc1234567890def",
            worktree_hash=None,
            scope="memory",
        )
        self.assertEqual(with_wt, without_wt)
        self.assertNotIn("worktrees", with_wt.parts)

    def test_files_scope_without_worktree_hash_raises(self):
        with self.assertRaises(ValueError):
            runner.resolve_db_path(
                repo_hash="abc1234567890def",
                worktree_hash=None,
                scope="files",
            )

    def test_unknown_scope_raises(self):
        with self.assertRaises(ValueError):
            runner.resolve_db_path(
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                scope="bogus",
            )


class CliArgumentTests(unittest.TestCase):
    """The argparse parser must accept the new flags."""

    def test_parse_args_accepts_repo_hash(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--query",
                "hello",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.repo_hash, "abc1234567890def")
            self.assertEqual(args.worktree_hash, "111122223333ffff")

    def test_parse_args_accepts_scope(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--scope",
                "files-docs",
                "--query",
                "x",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.scope, "files-docs")

    def test_parse_args_accepts_no_auto_build(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--query",
                "x",
                "--no-auto-build",
            ],
        ):
            args = runner.parse_args()
            self.assertTrue(args.no_auto_build)

    def test_parse_args_accepts_search_multi_scopes(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "search-multi",
                "--repo-hash",
                "abc1234567890def",
                "--query",
                "Git",
                "--scopes",
                "issues,specs,board,memory",
                "--no-auto-build",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.action, "search-multi")
            self.assertEqual(args.scopes, "issues,specs,board,memory")
            self.assertTrue(args.no_auto_build)

    def test_parse_args_accepts_respect_ttl(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-issues",
                "--repo-hash",
                "abc1234567890def",
                "--project-root",
                "/tmp/proj",
                "--respect-ttl",
            ],
        ):
            args = runner.parse_args()
            self.assertTrue(args.respect_ttl)

    def test_parse_args_accepts_mode(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--project-root",
                "/tmp/proj",
                "--mode",
                "incremental",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(args.mode, "incremental")

    def test_file_index_protocol_defaults_to_legacy(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--project-root",
                "/tmp/proj",
            ],
        ):
            args = runner.parse_args()
            self.assertEqual(
                getattr(args, "file_index_protocol", None),
                "legacy",
                "existing actions must keep legacy semantics unless v2 is explicit",
            )

    def test_parse_args_accepts_explicit_file_index_v2_protocol(self):
        with mock.patch.object(
            runner.sys,
            "argv",
            [
                "chroma_index_runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                "abc1234567890def",
                "--worktree-hash",
                "111122223333ffff",
                "--project-root",
                "/tmp/proj",
                "--file-index-protocol",
                "v2",
            ],
        ):
            try:
                args = runner.parse_args()
            except SystemExit as error:
                self.fail(
                    "Phase 71 explicit --file-index-protocol v2 is unsupported: "
                    f"{error}"
                )
            self.assertEqual(args.file_index_protocol, "v2")

    def test_invalid_explicit_v2_identity_is_bad_args_before_scan(self):
        with tempfile.TemporaryDirectory() as tmp:
            project = Path(tmp) / "project"
            project.mkdir()
            with mock.patch.object(
                runner.sys,
                "argv",
                [
                    "chroma_index_runner.py",
                    "--action",
                    "index-files",
                    "--repo-hash",
                    "../unsafe",
                    "--worktree-hash",
                    "111122223333ffff",
                    "--project-root",
                    str(project),
                    "--file-index-protocol",
                    "v2",
                ],
            ):
                args = runner.parse_args()

            with mock.patch.object(runner, "emit") as emit:
                with mock.patch.object(
                    runner, "_visible_file_records", return_value=[]
                ) as scan:
                    exit_code = runner._dispatch_v2(args.action, args)

        self.assertEqual(exit_code, 2)
        scan.assert_not_called()
        payload = emit.call_args.args[0]
        self.assertEqual(payload.get("error_code"), "BAD_ARGS", payload)


class ProtocolLayoutIntegrationTests(unittest.TestCase):
    def _tree_digest(self, root: Path) -> dict:
        self.assertTrue(root.is_dir(), root)
        return {
            path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(root.rglob("*"))
            if path.is_file()
        }

    def _run_index(
        self,
        project: Path,
        db_root: Path,
        coordinator: Path,
        protocol: str = "",
    ) -> dict:
        command = [
            sys.executable,
            str(Path(runner.__file__).resolve()),
            "--action",
            "index-files",
            "--project-root",
            str(project),
            "--repo-hash",
            "abc1234567890def",
            "--worktree-hash",
            "111122223333ffff",
            "--db-root",
            str(db_root),
        ]
        if protocol:
            command.extend(["--file-index-protocol", protocol])
        env = os.environ.copy()
        env.update(
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(coordinator),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            }
        )
        try:
            completed = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                env=env,
                timeout=30,
            )
        except subprocess.TimeoutExpired as error:
            self.fail(f"file index protocol subprocess timed out: {error}")
        self.assertEqual(
            completed.returncode,
            0,
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}",
        )
        lines = [line for line in completed.stdout.splitlines() if line.strip()]
        self.assertTrue(lines, "file index subprocess must emit a result")
        return json.loads(lines[-1])

    def _project(self, base: Path) -> Path:
        project = base / "project"
        src = project / "src"
        src.mkdir(parents=True)
        (src / "lib.rs").write_text("pub fn indexed() {}\n", encoding="utf-8")
        return project

    def test_default_protocol_writes_legacy_layout_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            coordinator = base / "coordinator"
            coordinator.mkdir()
            result = self._run_index(
                self._project(base), db_root, coordinator, protocol=""
            )

            self.assertTrue(result.get("ok"), result)
            legacy = runner.resolve_db_path(
                "abc1234567890def",
                "111122223333ffff",
                "files",
                db_root=db_root,
            )
            self.assertTrue(legacy.exists(), legacy)
            self.assertFalse(
                (db_root / "abc1234567890def" / "file-index-v2").exists(),
                "the default protocol must not materialize Phase 71 artifacts",
            )

    def test_explicit_v2_protocol_writes_additive_layout_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            coordinator = base / "coordinator"
            coordinator.mkdir()
            result = self._run_index(
                self._project(base), db_root, coordinator, protocol="v2"
            )

            self.assertTrue(result.get("ok"), result)
            v2_root = db_root / "abc1234567890def" / "file-index-v2"
            self.assertTrue(v2_root.is_dir(), v2_root)
            legacy = runner.resolve_db_path(
                "abc1234567890def",
                "111122223333ffff",
                "files",
                db_root=db_root,
            )
            self.assertFalse(
                legacy.exists(),
                "explicit v2 must not modify the legacy per-worktree store",
            )

    def test_legacy_and_v2_protocols_preserve_each_others_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            coordinator = base / "coordinator"
            coordinator.mkdir()
            project = self._project(base)
            legacy = runner.resolve_db_path(
                "abc1234567890def",
                "111122223333ffff",
                "files",
                db_root=db_root,
            )
            legacy_worktree_root = legacy.parent
            v2_root = db_root / "abc1234567890def" / "file-index-v2"

            legacy_result = self._run_index(
                project, db_root, coordinator, protocol=""
            )
            self.assertTrue(legacy_result.get("ok"), legacy_result)
            legacy_before_v2 = self._tree_digest(legacy_worktree_root)

            v2_result = self._run_index(project, db_root, coordinator, protocol="v2")
            self.assertTrue(v2_result.get("ok"), v2_result)
            self.assertEqual(
                self._tree_digest(legacy_worktree_root),
                legacy_before_v2,
                "explicit v2 must not rewrite legacy stores, generations, "
                "manifests, metadata, or active heads",
            )
            v2_before_legacy = self._tree_digest(v2_root)

            second_legacy = self._run_index(
                project, db_root, coordinator, protocol=""
            )
            self.assertTrue(second_legacy.get("ok"), second_legacy)
            self.assertEqual(
                self._tree_digest(v2_root),
                v2_before_legacy,
                "legacy refresh must not rewrite an existing v2 artifact or head",
            )


class ProbeCapabilityTests(unittest.TestCase):
    def test_probe_advertises_additive_file_index_protocols(self):
        result = runner.action_probe()
        self.assertTrue(result.get("ok"), result)
        self.assertIsInstance(result.get("file_index_protocols"), list, result)
        self.assertTrue(
            {"legacy", "v2"}.issubset(result["file_index_protocols"]),
            "probe must preserve legacy while advertising explicit file index v2",
        )


if __name__ == "__main__":
    unittest.main()
