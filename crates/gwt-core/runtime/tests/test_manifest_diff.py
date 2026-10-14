"""Phase 8: tests for manifest.json based incremental indexing.

Files index actions must persist (path, mtime, size) tuples and only re-embed
the diff on subsequent runs.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


def _make_repo(root: Path) -> None:
    (root / "src").mkdir(parents=True)
    (root / "src" / "a.rs").write_text("// a\nfn a() {}\n")
    (root / "src" / "b.rs").write_text("// b\nfn b() {}\n")
    (root / "src" / "c.rs").write_text("// c\nfn c() {}\n")
    (root / "src" / "d.rs").write_text("// d\nfn d() {}\n")
    (root / "src" / "e.rs").write_text("// e\nfn e() {}\n")


class ManifestHelperTests(unittest.TestCase):
    def test_helpers_exist(self):
        self.assertTrue(hasattr(runner, "read_manifest"))
        self.assertTrue(hasattr(runner, "write_manifest"))
        self.assertTrue(hasattr(runner, "compute_manifest_diff"))

    def test_write_then_read_round_trip(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp)
            entries = [
                {"path": "src/a.rs", "mtime": 1700000000, "size": 100},
                {"path": "src/b.rs", "mtime": 1700000001, "size": 200},
            ]
            runner.write_manifest(db, scope="files", entries=entries)
            loaded = runner.read_manifest(db, scope="files")
            self.assertEqual(loaded, entries)

    def test_read_manifest_returns_empty_when_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp)
            loaded = runner.read_manifest(db, scope="files")
            self.assertEqual(loaded, [])

    def test_compute_manifest_diff_detects_added(self):
        old = [{"path": "a", "mtime": 1, "size": 1}]
        new = [
            {"path": "a", "mtime": 1, "size": 1},
            {"path": "b", "mtime": 2, "size": 2},
        ]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["added"], ["b"])
        self.assertEqual(diff["changed"], [])
        self.assertEqual(diff["removed"], [])

    def test_compute_manifest_diff_detects_changed_by_mtime(self):
        old = [{"path": "a", "mtime": 1, "size": 100}]
        new = [{"path": "a", "mtime": 2, "size": 100}]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["changed"], ["a"])

    def test_compute_manifest_diff_detects_changed_by_size(self):
        old = [{"path": "a", "mtime": 1, "size": 100}]
        new = [{"path": "a", "mtime": 1, "size": 200}]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["changed"], ["a"])

    def test_compute_manifest_diff_detects_removed(self):
        old = [
            {"path": "a", "mtime": 1, "size": 1},
            {"path": "b", "mtime": 1, "size": 1},
        ]
        new = [{"path": "a", "mtime": 1, "size": 1}]
        diff = runner.compute_manifest_diff(old, new)
        self.assertEqual(diff["removed"], ["b"])


class IncrementalIndexingTests(unittest.TestCase):
    def test_full_index_writes_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _make_repo(root)

            result = runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="full",
                db_root=Path(tmp) / "index_root",
            )
            self.assertTrue(result["ok"], result)

            db = (
                Path(tmp)
                / "index_root"
                / "abc1234567890def"
                / "worktrees"
                / "111122223333ffff"
            )
            manifest = runner.read_manifest(db, scope="files")
            self.assertGreaterEqual(len(manifest), 5)
            for entry in manifest:
                self.assertIn("path", entry)
                self.assertIn("mtime", entry)
                self.assertIn("size", entry)

    def test_incremental_only_reembeds_changed_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _make_repo(root)
            db_root = Path(tmp) / "index_root"

            runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="full",
                db_root=db_root,
            )

            # Modify exactly one file. Sleep to ensure mtime changes.
            time.sleep(1.05)
            (root / "src" / "b.rs").write_text("// b modified\nfn b() {}\n")

            with mock.patch.object(
                runner, "embed_documents_for_paths", wraps=runner.embed_documents_for_paths
            ) as spy:
                result = runner.action_index_files_v2(
                    project_root=str(root),
                    repo_hash="abc1234567890def",
                    worktree_hash="111122223333ffff",
                    mode="incremental",
                    db_root=db_root,
                )
            self.assertTrue(result["ok"])
            # The spy must have been called with exactly one path.
            paths_passed = []
            for call in spy.call_args_list:
                paths_passed.extend(call[0][0])
            self.assertEqual(
                len(paths_passed),
                1,
                f"incremental should re-embed only the changed file, got {paths_passed}",
            )
            self.assertTrue(str(paths_passed[0]).endswith("b.rs"))

    def test_incremental_detects_deleted_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _make_repo(root)
            db_root = Path(tmp) / "index_root"

            runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="full",
                db_root=db_root,
            )

            (root / "src" / "c.rs").unlink()

            result = runner.action_index_files_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash="111122223333ffff",
                mode="incremental",
                db_root=db_root,
            )
            self.assertTrue(result["ok"])

            db = (
                db_root
                / "abc1234567890def"
                / "worktrees"
                / "111122223333ffff"
            )
            manifest = runner.read_manifest(db, scope="files")
            paths = {e["path"] for e in manifest}
            self.assertNotIn("src/c.rs", paths)


class Phase71OverlayManifestTests(unittest.TestCase):
    """AS-22/AS-23: one immutable overlay shadows both file scopes."""

    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        self.coordinator = self.base / "coordinator"
        self.coordinator.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(self.coordinator),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self._git("init", "--quiet", str(self.repo))
        self._git("-C", str(self.repo), "symbolic-ref", "HEAD", "refs/heads/develop")
        sources = {
            "src/keep.rs": "//! keep\nfn keep() {}\n",
            "src/change.rs": "//! before change\nfn change() {}\n",
            "src/delete.rs": "//! delete me\nfn delete_me() {}\n",
            "src/rename_old.rs": "//! rename me\nfn renamed() {}\n",
            "src/move_old.rs": "//! move scope\nfn moved() {}\n",
        }
        for relative, content in sources.items():
            path = self.repo / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        self._git("-C", str(self.repo), "add", ".")
        self._git(
            "-C",
            str(self.repo),
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "canonical base",
        )

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def _git(self, *args: str) -> None:
        env = os.environ.copy()
        env["GIT_CONFIG_NOSYSTEM"] = "1"
        env["GIT_CONFIG_GLOBAL"] = os.devnull
        env["GIT_ATTR_NOSYSTEM"] = "1"
        for key in (
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_PREFIX",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_SYSTEM",
        ):
            env.pop(key, None)
        for key in list(env):
            if (
                key == "GIT_CONFIG_COUNT"
                or key.startswith("GIT_CONFIG_KEY_")
                or key.startswith("GIT_CONFIG_VALUE_")
            ):
                env.pop(key)
        try:
            completed = subprocess.run(
                ["git", *args],
                check=False,
                capture_output=True,
                text=True,
                env=env,
                timeout=30,
            )
        except subprocess.TimeoutExpired as error:
            self.fail(f"git fixture command timed out: {error}")
        self.assertEqual(
            completed.returncode,
            0,
            f"git {' '.join(args)} failed\n{completed.stderr}",
        )

    def _mutate_visible_worktree(self) -> None:
        (self.repo / "src" / "change.rs").write_text(
            "//! after change\nfn change_v2() {}\n", encoding="utf-8"
        )
        (self.repo / "src" / "delete.rs").unlink()
        (self.repo / "src" / "rename_old.rs").rename(
            self.repo / "src" / "rename_new.rs"
        )
        (self.repo / "src" / "added.rs").write_text(
            "//! added\nfn added() {}\n", encoding="utf-8"
        )
        docs = self.repo / "docs"
        docs.mkdir()
        (self.repo / "src" / "move_old.rs").rename(docs / "move_new.md")

    def _build(self) -> dict:
        return runner.action_index_files_v2(
            project_root=str(self.repo),
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            mode="full",
            db_root=self.db_root,
            scope="files",
            file_index_protocol="v2",
        )

    def _overlay_descriptor(self, result: dict) -> dict:
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            REPO_HASH,
            WORKTREE_HASH,
            result["overlay_generation_id"],
            db_root=self.db_root,
        )
        return json.loads((overlay_dir / "descriptor.json").read_text(encoding="utf-8"))

    def _head_and_view(self) -> tuple[dict, dict]:
        worktree_root = (
            runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root)
            / "worktrees"
            / WORKTREE_HASH
        )
        head_path = worktree_root / "head.json"
        self.assertTrue(
            head_path.is_file(),
            "AS-23 requires one atomic Worktree View head for Files and FilesDocs",
        )
        head = json.loads(head_path.read_text(encoding="utf-8"))
        view_id = head.get("active_view_id")
        self.assertIsInstance(view_id, str, head)
        view_path = worktree_root / "views" / view_id / "descriptor.json"
        self.assertTrue(view_path.is_file(), f"active view is incomplete: {head}")
        return head, json.loads(view_path.read_text(encoding="utf-8"))

    def test_overlay_manifest_has_complete_shadow_set_for_all_diff_kinds(self):
        self._mutate_visible_worktree()
        result = self._build()
        self.assertTrue(result.get("ok"), result)
        overlay = self._overlay_descriptor(result)

        self.assertEqual(
            overlay["tombstones"],
            ["src/delete.rs", "src/move_old.rs", "src/rename_old.rs"],
        )
        self.assertEqual(
            overlay["files_shadow"],
            [
                "src/added.rs",
                "src/change.rs",
                "src/delete.rs",
                "src/move_old.rs",
                "src/rename_new.rs",
                "src/rename_old.rs",
            ],
            "add/change upserts and delete/rename tombstones form the complete Files shadow set",
        )
        self.assertEqual(
            overlay["files_docs_shadow"],
            ["docs/move_new.md"],
            "the new side of a Files-to-FilesDocs move must shadow in the destination scope",
        )

    def test_overlay_planner_classifies_inherited_upsert_tombstone_and_same_path_scope_move(self):
        planner = getattr(runner, "_plan_file_index_v2_overlay", None)
        self.assertTrue(
            callable(planner),
            "T-IDX-427 requires one overlay planner instead of an inline source-digest-only diff",
        )

        def record(path: str, digest: str, bucket: str) -> dict:
            return {
                "path": path,
                "source_digest": digest,
                "source_object": f"fixture:{digest}",
                "bucket": bucket,
                "document": f"Path: {path}\n{digest}",
                "metadata": {"path": path},
            }

        base = [
            record("src/inherited.rs", "same", "code"),
            record("src/change.rs", "old", "code"),
            record("src/delete.rs", "delete", "code"),
            record("src/rename_old.rs", "rename", "code"),
            record("shared/policy-record", "move", "code"),
        ]
        visible = [
            record("src/inherited.rs", "same", "code"),
            record("src/change.rs", "new", "code"),
            record("src/rename_new.rs", "rename", "code"),
            record("src/add.rs", "add", "code"),
            # A path-policy revision can move one logical path between scopes
            # without changing either its path or source bytes.
            record("shared/policy-record", "move", "docs"),
        ]
        plan = planner(base, visible)

        self.assertEqual(plan["inherited"], ["src/inherited.rs"])
        self.assertEqual(
            [item["path"] for item in plan["upserts"]],
            [
                "shared/policy-record",
                "src/add.rs",
                "src/change.rs",
                "src/rename_new.rs",
            ],
        )
        self.assertEqual(
            plan["tombstones"], ["src/delete.rs", "src/rename_old.rs"]
        )
        self.assertEqual(
            plan["files_shadow"],
            [
                "shared/policy-record",
                "src/add.rs",
                "src/change.rs",
                "src/delete.rs",
                "src/rename_new.rs",
                "src/rename_old.rs",
            ],
        )
        self.assertEqual(plan["files_docs_shadow"], ["shared/policy-record"])

    def test_scope_move_is_published_in_one_two_scope_view(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        baseline_head, _ = self._head_and_view()
        self._mutate_visible_worktree()
        result = self._build()
        self.assertTrue(result.get("ok"), result)
        head, view = self._head_and_view()
        overlay = self._overlay_descriptor(result)

        self.assertEqual(view.get("base_generation_id"), result["base_generation_id"])
        self.assertEqual(
            view.get("overlay_generation_id"), result["overlay_generation_id"]
        )
        self.assertEqual(view.get("visible_counts"), {"files": 4, "files-docs": 1})
        self.assertEqual(view.get("source_snapshot_id"), overlay["source_snapshot_id"])
        self.assertIn("src/move_old.rs", overlay["files_shadow"])
        self.assertIn("docs/move_new.md", overlay["files_docs_shadow"])
        self.assertEqual(head.get("sequence"), baseline_head.get("sequence") + 1)
        self.assertEqual(
            head.get("previous_view_id"),
            baseline_head.get("active_view_id"),
            "both scopes rotate through the same previous view",
        )

        worktree_root = (
            runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root)
            / "worktrees"
            / WORKTREE_HASH
        )
        view_dir = worktree_root / "views" / head["active_view_id"]
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            REPO_HASH,
            WORKTREE_HASH,
            result["overlay_generation_id"],
            db_root=self.db_root,
        )
        descriptor_path = overlay_dir / "descriptor.json"
        original = descriptor_path.read_bytes()

        def replace_required_shadow_with_unrelated_base(payload: dict) -> None:
            payload["files_shadow"].remove("src/move_old.rs")
            payload["files_shadow"].append("src/keep.rs")
            payload["files_shadow"].sort()

        def add_tombstone_to_wrong_scope(payload: dict) -> None:
            payload["files_docs_shadow"].append("src/delete.rs")
            payload["files_docs_shadow"].sort()

        def overlap_tombstone_and_upsert(payload: dict) -> None:
            payload["tombstones"].append("src/change.rs")
            payload["tombstones"].sort()

        for label, mutate in (
            ("scope move old-side shadow substitution", replace_required_shadow_with_unrelated_base),
            ("tombstone in opposite scope", add_tombstone_to_wrong_scope),
            ("tombstone overlapping an upsert", overlap_tombstone_and_upsert),
        ):
            with self.subTest(label=label):
                try:
                    payload = json.loads(original.decode("utf-8"))
                    mutate(payload)
                    descriptor_path.write_text(
                        json.dumps(
                            payload,
                            sort_keys=True,
                            separators=(",", ":"),
                            ensure_ascii=True,
                        ),
                        encoding="utf-8",
                    )
                    self.assertFalse(
                        runner._verify_file_index_v2_view(view_dir),
                        f"{label} must invalidate the complete shadow closure",
                    )
                finally:
                    descriptor_path.write_bytes(original)

    def test_manifest_scope_partition_rejects_one_path_in_both_scopes(self):
        parser = getattr(runner, "_validated_file_index_v2_manifest_scope_paths", None)
        self.assertTrue(
            callable(parser),
            "closure verification needs one cross-scope manifest partition validator",
        )

        def entry(path: str, scope: str) -> dict:
            digest = "a" * 64
            return {
                "path": path,
                "source_object": f"fixture:{path}",
                "source_digest": digest,
                "payload_digest": digest,
                "metadata_digest": digest,
                "scope": scope,
                "cas_key": digest,
                "input_digest": digest,
                "vector_checksum": digest,
                "dimension": 16,
            }

        self.assertIsNone(
            parser(
                [
                    entry("shared/path", "files"),
                    entry("shared/path", "files-docs"),
                ]
            ),
            "one artifact must never expose the same logical path in both collections",
        )
        self.assertFalse(runner._is_sorted_unique_string_list(["../escape.rs"]))
        self.assertTrue(
            runner._is_sorted_unique_string_list(["src/control-\u0085.rs"]),
            "UTF-8 Git paths only reject ASCII controls at the Rust/Python boundary",
        )

    def test_clean_tracked_checkout_eol_uses_git_object_as_inherited_source(self):
        filter_script = self.repo / "eol_filter.py"
        filter_script.write_text(
            "import sys\n"
            "data = sys.stdin.buffer.read()\n"
            "if sys.argv[1] == 'clean':\n"
            "    data = data.replace(b'\\r\\n', b'\\n')\n"
            "else:\n"
            "    data = data.replace(b'\\n', b'\\r\\n')\n"
            "sys.stdout.buffer.write(data)\n",
            encoding="utf-8",
        )
        (self.repo / ".gitattributes").write_text(
            "notes/*.txt filter=gwt-test-eol\n", encoding="utf-8"
        )
        quoted_python = subprocess.list2cmdline([sys.executable])
        quoted_script = subprocess.list2cmdline([str(filter_script)])
        self._git(
            "-C",
            str(self.repo),
            "config",
            "filter.gwt-test-eol.clean",
            f"{quoted_python} {quoted_script} clean",
        )
        self._git(
            "-C",
            str(self.repo),
            "config",
            "filter.gwt-test-eol.smudge",
            f"{quoted_python} {quoted_script} smudge",
        )
        self._git(
            "-C", str(self.repo), "config", "filter.gwt-test-eol.required", "true"
        )
        notes = self.repo / "notes"
        notes.mkdir()
        clean = notes / "clean.txt"
        clean.write_bytes(b"first line\nsecond line\n")
        self._git(
            "-C",
            str(self.repo),
            "add",
            ".gitattributes",
            "eol_filter.py",
            "notes/clean.txt",
        )
        self._git(
            "-C",
            str(self.repo),
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "tracked eol fixture",
        )

        # Force Git's configured smudge conversion instead of manually
        # writing bytes that Git could correctly classify as dirty.
        clean.unlink()
        self._git("-C", str(self.repo), "checkout-index", "--force", "notes/clean.txt")
        self.assertEqual(clean.read_bytes(), b"first line\r\nsecond line\r\n")
        self._git("-C", str(self.repo), "add", "notes/clean.txt")
        status = runner._git_command(
            self.repo,
            ["status", "--porcelain=v1", "--", "notes/clean.txt"],
        )
        self.assertEqual(status.returncode, 0, status.stderr)
        self.assertEqual(status.stdout, b"", status.stdout)

        result = self._build()
        self.assertTrue(result.get("ok"), result)
        overlay = self._overlay_descriptor(result)
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            REPO_HASH,
            WORKTREE_HASH,
            result["overlay_generation_id"],
            db_root=self.db_root,
        )
        manifest = json.loads((overlay_dir / "manifest.json").read_text(encoding="utf-8"))
        self.assertNotIn(
            "notes/clean.txt",
            [entry["path"] for entry in manifest["entries"]],
            "clean smudged bytes must inherit the pinned Git object instead of becoming an overlay",
        )
        self.assertNotIn("notes/clean.txt", overlay["files_docs_shadow"])

    def test_clean_branch_index_blob_different_from_canonical_is_overlay(self):
        self._git(
            "-C",
            str(self.repo),
            "update-ref",
            "refs/remotes/origin/develop",
            "HEAD",
        )
        changed = self.repo / "src" / "keep.rs"
        changed.write_text("//! branch version\nfn keep_on_branch() {}\n", encoding="utf-8")
        self._git("-C", str(self.repo), "add", "src/keep.rs")
        self._git(
            "-C",
            str(self.repo),
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "branch source",
        )

        result = self._build()
        self.assertTrue(result.get("ok"), result)
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            REPO_HASH,
            WORKTREE_HASH,
            result["overlay_generation_id"],
            db_root=self.db_root,
        )
        manifest = json.loads((overlay_dir / "manifest.json").read_text(encoding="utf-8"))
        self.assertIn(
            "src/keep.rs",
            [entry["path"] for entry in manifest["entries"]],
            "a clean branch/index blob that differs from the canonical ref belongs to Overlay",
        )

    def test_v2_source_capture_io_errors_never_replace_the_head(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        worktree_root = (
            runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root)
            / "worktrees"
            / WORKTREE_HASH
        )
        head_path = worktree_root / "head.json"
        original_head = head_path.read_bytes()
        unreadable = (self.repo / "src" / "unreadable.rs").resolve()
        unreadable.write_text("fn unreadable() {}\n", encoding="utf-8")

        real_walk = os.walk
        real_stat = Path.stat
        real_read_bytes = Path.read_bytes

        def walk_failure(*args, **kwargs):
            if "onerror" not in kwargs:
                return real_walk(*args, **kwargs)
            onerror = kwargs["onerror"]
            onerror(PermissionError("walk denied"))
            return iter(())

        def stat_failure(path: Path, *args, **kwargs):
            if path == unreadable:
                raise PermissionError("stat denied")
            return real_stat(path, *args, **kwargs)

        def read_failure(path: Path, *args, **kwargs):
            if path == unreadable:
                raise PermissionError("read denied")
            return real_read_bytes(path, *args, **kwargs)

        for label, patcher in (
            ("walk", mock.patch.object(runner.os, "walk", side_effect=walk_failure)),
            ("stat", mock.patch.object(type(unreadable), "stat", stat_failure)),
            ("read", mock.patch.object(type(unreadable), "read_bytes", read_failure)),
        ):
            with self.subTest(operation=label), patcher:
                with self.assertRaisesRegex(RuntimeError, "file-index-v2 source"):
                    self._build()
                self.assertEqual(
                    head_path.read_bytes(),
                    original_head,
                    f"{label} failure must not publish a partial visible corpus",
                )

        # Keep references visibly used so accidental fixture refactors do not
        # silently change the mocked traversal contract.
        self.assertTrue(callable(real_walk))


if __name__ == "__main__":
    unittest.main()
