"""Phase 70 T-IDX-391 (Issue #3264): atomic generation store fault contract.

FR-390: full rebuilds construct an immutable staging generation and publish
it by atomically replacing the `active.json` pointer. The live store is
never reset in place: kill / crash / disk full at any boundary leaves the
previous healthy generation searchable, and an incomplete generation is
never exposed to readers. Legacy stores stay readable and lazily migrate on
the next update (AS-17); abandoned generations older than 24 hours are the
only GC target.
"""

from __future__ import annotations

import hashlib
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

RUNNER_PATH = Path(runner.__file__).resolve()

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


def _make_project(base: Path, docs: int) -> Path:
    project = base / "project"
    src = project / "src"
    src.mkdir(parents=True, exist_ok=True)
    for index in range(docs):
        (src / f"module_{index:02}.rs").write_text(
            f"//! module {index}\nfn feature_{index}() {{}}\n", encoding="utf-8"
        )
    return project


class GenerationStoreTests(unittest.TestCase):
    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        self.coord = self.base / "coordinator"
        self.coord.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(self.coord),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def _build(self, project: Path) -> dict:
        return runner.action_index_files_v2(
            project_root=str(project),
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            mode="full",
            db_root=self.db_root,
            scope="files",
        )

    def _db_path(self) -> Path:
        return runner.resolve_db_path(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )

    def _status(self) -> dict:
        return runner._scope_status_v2(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )

    def _search(self, query: str) -> dict:
        return runner._search_scope_collection(
            REPO_HASH,
            WORKTREE_HASH,
            "files",
            query,
            5,
            "semantic",
            self.db_root,
            None,
        )

    def test_publish_creates_generation_with_atomic_active_pointer(self):
        project = _make_project(self.base, 6)
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)

        db_path = self._db_path()
        pointer = runner.active_pointer_path(db_path)
        self.assertTrue(
            pointer.is_file(),
            f"publish must write the active.json pointer, missing at {pointer}",
        )
        active = json.loads(pointer.read_text(encoding="utf-8"))
        generation_dir = pointer.parent / active["generation"]
        self.assertTrue(
            (generation_dir / "chroma.sqlite3").exists(),
            f"active pointer must reference a complete generation: {active}",
        )
        resolved = runner.resolve_active_store(db_path)
        self.assertEqual(
            resolved,
            generation_dir,
            "readers must resolve through the active pointer",
        )
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 6, status)
        self.assertTrue(self._search("module feature").get("results"), "search works")

    def test_kill_during_staging_build_preserves_active_generation(self):
        project = _make_project(self.base, 48)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        # Change every document so the rebuild has real embedding work
        # (unchanged records would be reused without checkpoints, FR-391).
        for index in range(48):
            (project / "src" / f"module_{index:02}.rs").write_text(
                f"//! module {index} v2\nfn feature_{index}_v2() {{}}\n",
                encoding="utf-8",
            )

        home = self.base / "home"
        home.mkdir(exist_ok=True)
        env = os.environ.copy()
        env["HOME"] = str(home)
        env["USERPROFILE"] = str(home)
        env["GWT_INDEX_FAKE_EMBEDDING"] = "1"
        env["GWT_INDEX_COORDINATOR_ROOT"] = str(self.coord)
        proc = subprocess.Popen(
            [
                sys.executable,
                str(RUNNER_PATH),
                "--action",
                "index-files",
                "--repo-hash",
                REPO_HASH,
                "--worktree-hash",
                WORKTREE_HASH,
                "--project-root",
                str(project),
                "--mode",
                "full",
                "--scope",
                "files",
                "--qos",
                "background",
                "--db-root",
                str(self.db_root),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        # Kill the rebuild after its first checkpoint (mid-staging-build).
        # The stderr reader runs on a watchdog thread so a hung runner can
        # never block the test past its deadline (PR #3301 review).
        import queue
        import threading

        assert proc.stderr is not None
        lines: "queue.Queue[str]" = queue.Queue()

        def _pump(stream, sink):
            for line in stream:
                sink.put(line)

        reader = threading.Thread(
            target=_pump, args=(proc.stderr, lines), daemon=True
        )
        reader.start()
        killed = False
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                line = lines.get(timeout=0.5)
            except queue.Empty:
                if proc.poll() is not None:
                    break
                continue
            if '"done": 16' in line or '"done":16' in line:
                proc.kill()
                killed = True
                break
        if not killed:
            proc.kill()
        proc.wait(timeout=30)
        self.assertTrue(killed, "expected to kill the rebuild at a checkpoint")

        # AS-9: the previous healthy generation stays active and searchable.
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 48, status)
        self.assertTrue(self._search("module feature").get("results"))

        # A follow-up rebuild completes normally.
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("indexed"), 48, result)

    def test_disk_full_during_publish_keeps_previous_generation(self):
        project = _make_project(self.base, 6)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        (project / "src" / "module_00.rs").write_text(
            "//! module 0 changed\nfn feature_0_changed() {}\n", encoding="utf-8"
        )
        real_replace = os.replace

        def failing_replace(src, dst, *args, **kwargs):
            if str(dst).endswith("active.json"):
                raise OSError(28, "No space left on device")
            return real_replace(src, dst, *args, **kwargs)

        with mock.patch("os.replace", side_effect=failing_replace):
            result = self._build(project)

        self.assertFalse(
            result.get("ok"),
            f"a failed publish must not report silent success: {result}",
        )
        self.assertEqual(result.get("error_code"), "PUBLISH_FAILED", result)
        # AS-9: previous generation still active and searchable.
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 6, status)

    def test_corrupt_active_pointer_is_classified_for_repair_not_crash(self):
        project = _make_project(self.base, 4)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        pointer = runner.active_pointer_path(self._db_path())
        pointer.write_text("{not valid json", encoding="utf-8")

        status = self._status()
        self.assertTrue(
            status["repair_required"],
            f"corrupt active pointer must classify as repair-required: {status}",
        )

        # Rebuild repairs the pointer.
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 4, status)

    def test_legacy_layout_serves_reads_and_migrates_on_next_update(self):
        # AS-17: a pre-generation store (chroma directly under the scope dir)
        # keeps serving reads without a rebuild and migrates on next update.
        project = _make_project(self.base, 3)
        db_path = self._db_path()
        db_path.mkdir(parents=True, exist_ok=True)
        client, collection = runner._make_chroma_collection(
            db_path, runner.V2_FILES_CODE_COLLECTION
        )
        try:
            paths = sorted((project / "src").glob("*.rs"))
            runner.embed_documents_for_paths(paths, project, collection)
        finally:
            runner._close_chroma_client(client)
        entries = runner._build_manifest_entries(
            project, sorted((project / "src").glob("*.rs"))
        )
        runner.write_manifest(db_path, scope="files", entries=entries)
        runner._write_scope_meta(
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            scope="files",
            db_root=self.db_root,
            updates={"document_count": 3},
        )

        status = self._status()
        self.assertTrue(status["healthy"], f"legacy store must stay readable: {status}")
        self.assertEqual(status["document_count"], 3, status)
        self.assertTrue(self._search("module feature").get("results"))

        # Next update publishes a generation without a startup mass rebuild.
        result = self._build(project)
        self.assertTrue(result.get("ok"), result)
        self.assertTrue(runner.active_pointer_path(db_path).is_file())
        status = self._status()
        self.assertTrue(status["healthy"], status)
        self.assertEqual(status["document_count"], 3, status)

    def test_abandoned_generations_are_gc_only_after_24_hours(self):
        project = _make_project(self.base, 3)
        baseline = self._build(project)
        self.assertTrue(baseline.get("ok"), baseline)

        pointer = runner.active_pointer_path(self._db_path())
        generations_root = pointer.parent
        old_abandoned = generations_root / "gen-old-abandoned"
        old_abandoned.mkdir()
        (old_abandoned / "chroma.sqlite3").write_text("old", encoding="utf-8")
        stale_time = time.time() - 25 * 3600
        os.utime(old_abandoned, (stale_time, stale_time))
        fresh_abandoned = generations_root / "gen-fresh-abandoned"
        fresh_abandoned.mkdir()
        (fresh_abandoned / "chroma.sqlite3").write_text("fresh", encoding="utf-8")

        result = self._build(project)
        self.assertTrue(result.get("ok"), result)

        self.assertFalse(
            old_abandoned.exists(),
            "generations abandoned for more than 24h must be garbage collected",
        )
        self.assertTrue(
            fresh_abandoned.exists(),
            "recently abandoned generations must be retained (crash recovery)",
        )
        active = json.loads(pointer.read_text(encoding="utf-8"))
        self.assertTrue((generations_root / active["generation"]).exists())


class Phase71WorktreeViewPublicationTests(unittest.TestCase):
    """AS-23/AS-25: publish a verified two-scope view, never a partial pair."""

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
        (self.repo / "src").mkdir()
        (self.repo / "docs").mkdir()
        (self.repo / "src" / "feature.rs").write_text(
            "//! feature v1\nfn feature() {}\n", encoding="utf-8"
        )
        (self.repo / "docs" / "guide.md").write_text(
            "# Guide v1\n", encoding="utf-8"
        )
        self._commit("canonical v1")

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

    def _commit(self, message: str) -> None:
        self._git("-C", str(self.repo), "add", "-A")
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
            message,
        )

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

    def _worktree_root(self) -> Path:
        return (
            runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root)
            / "worktrees"
            / WORKTREE_HASH
        )

    def _head_path(self) -> Path:
        return self._worktree_root() / "head.json"

    def _head(self) -> dict:
        path = self._head_path()
        self.assertTrue(path.is_file(), "v2 build must publish a Worktree View head")
        return json.loads(path.read_text(encoding="utf-8"))

    def _view_descriptor(self, view_id: str) -> dict:
        path = self._worktree_root() / "views" / view_id / "descriptor.json"
        self.assertTrue(path.is_file(), f"view {view_id} is not materialized")
        return json.loads(path.read_text(encoding="utf-8"))

    def _expected_head_checksum(self, head: dict) -> str:
        canonical = {
            "schema_version": head["schema_version"],
            "active_view_id": head["active_view_id"],
            "previous_view_id": head["previous_view_id"],
            "sequence": head["sequence"],
        }
        return hashlib.sha256(
            json.dumps(
                canonical,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=True,
            ).encode("utf-8")
        ).hexdigest()

    def test_verified_view_publish_rotates_active_and_previous_atomically(self):
        first = self._build()
        self.assertTrue(first.get("ok"), first)
        first_head = self._head()
        first_active = first_head.get("active_view_id")
        self.assertIsInstance(first_active, str, first_head)
        self.assertEqual(first_head.get("sequence"), 1, first_head)
        self.assertIsNone(first_head.get("previous_view_id"), first_head)
        self.assertEqual(first_head.get("checksum"), self._expected_head_checksum(first_head))
        first_head_bytes = self._head_path().read_bytes()

        repeated = self._build()
        self.assertTrue(repeated.get("ok"), repeated)
        self.assertEqual(
            self._head_path().read_bytes(),
            first_head_bytes,
            "publishing the same immutable pair must be an idempotent no-op",
        )

        (self.repo / "src" / "overlay.rs").write_text(
            "//! overlay\nfn overlay() {}\n", encoding="utf-8"
        )
        second = self._build()
        self.assertTrue(second.get("ok"), second)
        second_head = self._head()

        self.assertEqual(
            second_head.get("previous_view_id"),
            first_active,
            "head rotation must retain the exact previously active view",
        )
        self.assertNotEqual(second_head.get("active_view_id"), first_active)
        self.assertEqual(
            second_head.get("checksum"), self._expected_head_checksum(second_head)
        )
        active_id = second_head["active_view_id"]
        descriptor = self._view_descriptor(active_id)
        self.assertEqual(descriptor["view_id"], active_id)
        self.assertEqual(descriptor["base_generation_id"], second["base_generation_id"])
        self.assertEqual(
            descriptor["overlay_generation_id"], second["overlay_generation_id"]
        )
        self.assertEqual(descriptor["visible_counts"], {"files": 2, "files-docs": 1})

    def test_head_replace_runs_only_after_base_and_overlay_artifacts_finish(self):
        events = []
        real_materialize = runner._materialize_file_artifact_pair
        real_replace = os.replace

        def record_materialize(*args, **kwargs):
            result = real_materialize(*args, **kwargs)
            events.append(f"artifact:{args[4]['kind']}:done")
            return result

        def record_replace(src, dst, *args, **kwargs):
            if Path(dst) == self._head_path():
                candidate_head = json.loads(Path(src).read_text(encoding="utf-8"))
                candidate_view = (
                    self._worktree_root()
                    / "views"
                    / candidate_head["active_view_id"]
                )
                self.assertTrue(
                    (candidate_view / "descriptor.json").is_file(),
                    "a head must never point at an unmaterialized View",
                )
                verify = getattr(runner, "_verify_file_index_v2_view", None)
                self.assertTrue(callable(verify), "head publish requires closure verification")
                self.assertTrue(
                    verify(candidate_view),
                    "the complete View closure must verify before head replace",
                )
                events.append("view:verified")
                events.append("head:replace")
            return real_replace(src, dst, *args, **kwargs)

        with mock.patch.object(
            runner, "_materialize_file_artifact_pair", side_effect=record_materialize
        ):
            with mock.patch("os.replace", side_effect=record_replace):
                result = self._build()

        self.assertTrue(result.get("ok"), result)
        self.assertIn("head:replace", events, events)
        self.assertLess(events.index("artifact:base:done"), events.index("head:replace"))
        self.assertLess(events.index("artifact:overlay:done"), events.index("head:replace"))
        self.assertLess(events.index("view:verified"), events.index("head:replace"))

    def test_head_sequence_exhaustion_preserves_old_head_bytes(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        exhausted = self._head()
        exhausted["sequence"] = 2**64 - 1
        exhausted["checksum"] = self._expected_head_checksum(exhausted)
        self._head_path().write_text(
            json.dumps(exhausted, sort_keys=True, separators=(",", ":")),
            encoding="utf-8",
        )
        old_head = self._head_path().read_bytes()

        (self.repo / "src" / "new-view.rs").write_text(
            "fn new_view() {}\n", encoding="utf-8"
        )
        result = self._build()

        self.assertFalse(result.get("ok"), result)
        self.assertEqual(result.get("error_code"), "PUBLISH_FAILED", result)
        self.assertFalse(result.get("retryable"), result)
        self.assertEqual(self._head_path().read_bytes(), old_head)

        overflow = dict(exhausted, sequence=2**64)
        overflow["checksum"] = self._expected_head_checksum(overflow)
        self.assertFalse(runner._file_index_v2_head_is_valid(overflow))

    @unittest.skipIf(os.name == "nt", "POSIX directory fsync contract")
    def test_head_writer_fsyncs_same_directory_temp_and_parent_around_replace(self):
        writer = getattr(runner, "_write_file_index_v2_head_atomic", None)
        self.assertTrue(
            callable(writer),
            "T-IDX-427 requires a durable same-directory Worktree View head writer",
        )
        head_path = self._head_path()
        head_path.parent.mkdir(parents=True)
        head_path.write_bytes(b'{"old":true}')
        events = []
        replacements = []
        real_fsync = os.fsync
        real_replace = os.replace

        def record_fsync(fd):
            events.append("fsync")
            return real_fsync(fd)

        def record_replace(src, dst, *args, **kwargs):
            events.append("replace")
            replacements.append((Path(src), Path(dst)))
            return real_replace(src, dst, *args, **kwargs)

        payload = {
            "schema_version": 1,
            "active_view_id": "view-a",
            "previous_view_id": None,
            "sequence": 1,
        }
        payload["checksum"] = self._expected_head_checksum(payload)
        with mock.patch("os.fsync", side_effect=record_fsync):
            with mock.patch("os.replace", side_effect=record_replace):
                writer(head_path, payload)

        self.assertEqual(len(replacements), 1, replacements)
        source, destination = replacements[0]
        self.assertEqual(destination, head_path)
        self.assertEqual(source.parent, head_path.parent)
        replace_index = events.index("replace")
        self.assertIn("fsync", events[:replace_index], events)
        self.assertIn("fsync", events[replace_index + 1 :], events)
        self.assertEqual(json.loads(head_path.read_text(encoding="utf-8")), payload)

    def test_overlay_failure_after_new_base_keeps_old_pinned_view(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        self._head()
        old_head = self._head_path().read_bytes()
        old_views = set((self._worktree_root() / "views").iterdir())

        (self.repo / "src" / "feature.rs").write_text(
            "//! feature v2\nfn feature_v2() {}\n", encoding="utf-8"
        )
        self._commit("canonical v2")
        real_materialize = runner._materialize_file_artifact_pair
        published_bases = []

        def fail_after_base(*args, **kwargs):
            descriptor = args[4]
            result = real_materialize(*args, **kwargs)
            if descriptor.get("kind") == "base":
                published_bases.append(Path(args[0]))
                return result
            raise RuntimeError("injected overlay failure after base publish")

        with mock.patch.object(
            runner, "_materialize_file_artifact_pair", side_effect=fail_after_base
        ):
            with self.assertRaisesRegex(RuntimeError, "injected overlay failure"):
                self._build()

        self.assertTrue(published_bases and published_bases[-1].is_dir())
        self.assertEqual(self._head_path().read_bytes(), old_head)
        old_active = json.loads(old_head)["active_view_id"]
        old_view = self._view_descriptor(old_active)
        self.assertEqual(
            old_view["base_generation_id"], baseline["base_generation_id"]
        )
        self.assertEqual(
            set((self._worktree_root() / "views").iterdir()),
            old_views,
            "base-only or overlay-only state must never create a partial view",
        )

    def test_source_superseded_during_build_retains_artifacts_without_head_switch(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        self._head()
        old_head = self._head_path().read_bytes()
        old_overlay_dirs = set((self._worktree_root() / "overlays").iterdir())
        source = self.repo / "src" / "feature.rs"
        staged_bytes = b"//! dirty v2\nfn dirty_v2() {}\n"
        source.write_bytes(staged_bytes)
        real_materialize = runner._materialize_file_artifact_pair
        mutated = False

        def mutate_after_overlay(*args, **kwargs):
            nonlocal mutated
            result = real_materialize(*args, **kwargs)
            if args[4].get("kind") == "overlay" and not mutated:
                source.write_text(
                    "//! dirty v3 arrived during build\nfn dirty_v3() {}\n",
                    encoding="utf-8",
                )
                mutated = True
            return result

        with mock.patch.object(
            runner, "_materialize_file_artifact_pair", side_effect=mutate_after_overlay
        ):
            result = self._build()

        self.assertTrue(mutated, "fixture must supersede the immutable source snapshot")
        self.assertTrue(result.get("ok"), result)
        self.assertIs(result.get("superseded"), True, result)
        self.assertIs(result.get("published"), False, result)
        self.assertEqual(self._head_path().read_bytes(), old_head)
        self.assertGreater(
            len(set((self._worktree_root() / "overlays").iterdir())),
            len(old_overlay_dirs),
            "verified immutable artifacts and CAS work remain reusable for the follow-up",
        )
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            REPO_HASH,
            WORKTREE_HASH,
            result["overlay_generation_id"],
            db_root=self.db_root,
        )
        manifest = json.loads(
            (overlay_dir / "manifest.json").read_text(encoding="utf-8")
        )
        feature = next(
            entry for entry in manifest["entries"] if entry["path"] == "src/feature.rs"
        )
        self.assertEqual(feature["source_digest"], hashlib.sha256(staged_bytes).hexdigest())
        cas_path = (
            runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root)
            / "cas"
            / feature["cas_key"][:2]
            / f"{feature['cas_key']}.json"
        )
        self.assertTrue(cas_path.is_file(), "superseded snapshot CAS must be retained")

    def test_head_replace_permission_error_preserves_old_head_bytes(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        self._head()
        old_head = self._head_path().read_bytes()
        (self.repo / "src" / "overlay.rs").write_text(
            "//! second view\nfn second_view() {}\n", encoding="utf-8"
        )
        real_replace = os.replace

        def deny_head_replace(src, dst, *args, **kwargs):
            if Path(dst) == self._head_path():
                self.assertEqual(
                    self._head_path().read_bytes(),
                    old_head,
                    "head publication must not unlink or rename the old head before replace",
                )
                raise PermissionError(13, "simulated Windows open-handle denial")
            return real_replace(src, dst, *args, **kwargs)

        with mock.patch("os.replace", side_effect=deny_head_replace):
            result = self._build()

        self.assertFalse(result.get("ok"), result)
        self.assertEqual(result.get("error_code"), "PUBLISH_FAILED", result)
        self.assertEqual(self._head_path().read_bytes(), old_head)


if __name__ == "__main__":
    unittest.main()
