"""Phase 70 T-IDX-392 (Issue #3264): incremental generation build contract.

FR-391: additions / changes / deletions are detected through stable record
IDs and content hashes; unchanged records reuse their existing embeddings
from the previous healthy generation, only changed records are encoded, and
deleted IDs never reach the new generation. FR-392 / AS-10: when the source
changes between staging and publish, the late revalidation aborts the
publish and the active pointer stays on the previous generation.
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Optional
from unittest import mock

import chroma_index_runner as runner

REPO_HASH = "abc1234567890def"
SECOND_REPO_HASH = "fedcba0987654321"
WORKTREE_HASH = "111122223333ffff"
SECOND_WORKTREE_HASH = "aaaabbbbccccdddd"


class CompatibilityDescriptorTests(unittest.TestCase):
    def _descriptor(self) -> dict:
        return {
            "layout_version": 2,
            "index_schema_version": 1,
            "scope_set": ["files", "files-docs"],
            "model_id": "intfloat/multilingual-e5-base",
            "model_revision": "model-revision-a",
            "dimension": 768,
            "normalization": "none",
            "metric": "cosine",
            "query_prefix": "query: ",
            "passage_prefix": "passage: ",
            "document_contract": {
                "payload_builder_version": 1,
                "decode": "utf-8-replace",
                "content_limit": 2000,
            },
            "path_policy_hash": "path-policy-a",
            "writer_protocol": "file-index-v2",
            "runner_hash": "runner-a",
        }

    def _compatibility(self, left: dict, right: dict) -> bool:
        compatibility = getattr(runner, "file_index_compatibility", None)
        self.assertIsNotNone(
            compatibility,
            "Phase 71 requires pure file_index_compatibility descriptor checking",
        )
        return compatibility(left, right)

    def test_semantic_descriptor_mismatches_are_incompatible(self):
        self.assertTrue(
            callable(getattr(runner, "file_index_compatibility", None)),
            "Phase 71 requires pure file_index_compatibility descriptor checking",
        )
        changes = {
            "layout_version": 3,
            "index_schema_version": 2,
            "scope_set": ["files"],
            "model_id": "different-model",
            "model_revision": "model-revision-b",
            "dimension": 384,
            "normalization": "l2",
            "metric": "dot",
            "query_prefix": "search: ",
            "passage_prefix": "document: ",
            "path_policy_hash": "path-policy-b",
            "writer_protocol": "file-index-v3",
        }
        baseline = self._descriptor()

        for field, replacement in changes.items():
            with self.subTest(field=field):
                candidate = copy.deepcopy(baseline)
                candidate[field] = replacement
                self.assertFalse(
                    self._compatibility(baseline, candidate),
                    f"{field} mismatch must reject artifact reuse",
                )
                self.assertFalse(
                    self._compatibility(candidate, baseline),
                    f"{field} compatibility must be symmetric",
                )

        document_changes = {
            "payload_builder_version": 2,
            "decode": "utf-16-strict",
            "content_limit": 1000,
        }
        for field, replacement in document_changes.items():
            with self.subTest(field=f"document_contract.{field}"):
                candidate = copy.deepcopy(baseline)
                candidate["document_contract"][field] = replacement
                self.assertFalse(
                    self._compatibility(baseline, candidate),
                    f"document_contract.{field} must invalidate artifact reuse",
                )
                self.assertFalse(
                    self._compatibility(candidate, baseline),
                    f"document_contract.{field} compatibility must be symmetric",
                )

    def test_runner_hash_alone_does_not_invalidate_compatibility(self):
        baseline = self._descriptor()
        candidate = copy.deepcopy(baseline)
        candidate["runner_hash"] = "runner-b"

        self.assertTrue(
            self._compatibility(baseline, candidate),
            "runner_hash is provenance, not a semantic compatibility key",
        )


class _IncrementalFixture(unittest.TestCase):
    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        coord = self.base / "coordinator"
        coord.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(coord),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()
        self.project = self.base / "project"
        self.src = self.project / "src"
        self.src.mkdir(parents=True)
        for index in range(8):
            self._write_doc(index, f"//! module {index}\nfn feature_{index}() {{}}\n")

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def _write_doc(self, index: int, body: str) -> Path:
        path = self.src / f"module_{index:02}.rs"
        path.write_text(body, encoding="utf-8")
        return path

    def _make_worktree_project(
        self, name: str, overlay_index: Optional[int] = None
    ) -> Path:
        project = self.base / name
        src = project / "src"
        src.mkdir(parents=True)
        for index in range(8):
            (src / f"module_{index:02}.rs").write_text(
                f"//! module {index}\nfn feature_{index}() {{}}\n",
                encoding="utf-8",
            )
        if overlay_index is not None:
            (src / f"overlay_{overlay_index:02}.rs").write_text(
                f"//! unique overlay {overlay_index}\n"
                f"fn overlay_{overlay_index}() {{}}\n",
                encoding="utf-8",
            )
        return project

    def _run_git(self, *args: str) -> None:
        git_env = os.environ.copy()
        git_env["GIT_CONFIG_NOSYSTEM"] = "1"
        git_env["GIT_CONFIG_GLOBAL"] = os.devnull
        git_env["GIT_ATTR_NOSYSTEM"] = "1"
        git_env.pop("GIT_CONFIG_PARAMETERS", None)
        git_env.pop("GIT_CONFIG_SYSTEM", None)
        for key in list(git_env):
            if (
                key == "GIT_CONFIG_COUNT"
                or key.startswith("GIT_CONFIG_KEY_")
                or key.startswith("GIT_CONFIG_VALUE_")
            ):
                git_env.pop(key)
        try:
            completed = subprocess.run(
                ["git", *args],
                check=False,
                capture_output=True,
                text=True,
                env=git_env,
                timeout=30,
            )
        except subprocess.TimeoutExpired as error:
            self.fail(f"git fixture command timed out: {error}")
        self.assertEqual(
            completed.returncode,
            0,
            f"git {' '.join(args)} failed\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}",
        )

    def _make_canonical_repo(self) -> Path:
        repo = self.base / "canonical-repo"
        repo.mkdir()
        self._run_git("init", "--quiet", str(repo))
        self._run_git("-C", str(repo), "symbolic-ref", "HEAD", "refs/heads/develop")
        src = repo / "src"
        src.mkdir()
        for index in range(8):
            (src / f"module_{index:02}.rs").write_text(
                f"//! module {index}\nfn feature_{index}() {{}}\n",
                encoding="utf-8",
            )
        self._run_git("-C", str(repo), "add", "src")
        self._run_git(
            "-C",
            str(repo),
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "canonical base",
        )
        return repo

    def _clone_canonical_worktree(
        self, canonical_repo: Path, name: str, overlay_index: int
    ) -> Path:
        project = self.base / name
        self._run_git("clone", "--quiet", str(canonical_repo), str(project))
        (project / "src" / f"overlay_{overlay_index:02}.rs").write_text(
            f"//! unique overlay {overlay_index}\n"
            f"fn overlay_{overlay_index}() {{}}\n",
            encoding="utf-8",
        )
        return project

    def _compatibility_descriptor(self) -> dict:
        return {
            "layout_version": 2,
            "index_schema_version": 1,
            "scope_set": ["files", "files-docs"],
            "model_id": "intfloat/multilingual-e5-base",
            "model_revision": "model-revision-a",
            "dimension": 768,
            "normalization": "none",
            "metric": "cosine",
            "query_prefix": "query: ",
            "passage_prefix": "passage: ",
            "document_contract": {
                "payload_builder_version": 1,
                "decode": "utf-8-replace",
                "content_limit": 2000,
            },
            "path_policy_hash": "path-policy-a",
            "writer_protocol": "file-index-v2",
            "runner_hash": "runner-a",
        }

    def _physical_collection_count(self, artifact_dir: Path) -> int:
        stores = [path.parent for path in artifact_dir.rglob("chroma.sqlite3")]
        self.assertEqual(
            len(stores),
            1,
            f"one files vector store is required under {artifact_dir}: {stores}",
        )
        client, collection = runner._open_chroma_collection(
            stores[0], runner.V2_FILES_CODE_COLLECTION
        )
        try:
            return int(collection.count())
        finally:
            runner._close_chroma_client(client)

    def _build_protocol_v2(
        self,
        project: Path,
        worktree_hash: str,
        repo_hash: str = REPO_HASH,
        compatibility_descriptor: Optional[dict] = None,
    ) -> dict:
        try:
            arguments = {
                "project_root": str(project),
                "repo_hash": repo_hash,
                "worktree_hash": worktree_hash,
                "mode": "full",
                "db_root": self.db_root,
                "scope": "files",
                "file_index_protocol": "v2",
            }
            if compatibility_descriptor is not None:
                arguments["compatibility_descriptor"] = compatibility_descriptor
            return runner.action_index_files_v2(
                **arguments,
            )
        except TypeError as error:
            self.fail(
                "Phase 71 explicit v2 file index protocol is not implemented: "
                f"{error}"
            )

    def _run_protocol_v2_subprocess(
        self,
        project: Path,
        worktree_hash: str,
        repo_hash: str = REPO_HASH,
    ) -> dict:
        try:
            completed = subprocess.run(
                [
                    sys.executable,
                    str(Path(runner.__file__).resolve()),
                    "--action",
                    "index-files",
                    "--project-root",
                    str(project),
                    "--repo-hash",
                    repo_hash,
                    "--worktree-hash",
                    worktree_hash,
                    "--db-root",
                    str(self.db_root),
                    "--file-index-protocol",
                    "v2",
                ],
                check=False,
                capture_output=True,
                text=True,
                env=os.environ.copy(),
                timeout=30,
            )
        except subprocess.TimeoutExpired as error:
            self.fail(f"Phase 71 explicit v2 subprocess timed out: {error}")
        self.assertEqual(
            completed.returncode,
            0,
            "Phase 71 explicit v2 subprocess failed\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}",
        )
        output_lines = [line for line in completed.stdout.splitlines() if line.strip()]
        self.assertTrue(output_lines, "v2 subprocess must emit a result envelope")
        try:
            return json.loads(output_lines[-1])
        except json.JSONDecodeError as error:
            self.fail(
                "v2 subprocess did not end with a JSON result envelope: "
                f"{error}\nstdout:\n{completed.stdout}"
            )

    def _build(self) -> dict:
        return runner.action_index_files_v2(
            project_root=str(self.project),
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

    def _store_ids(self) -> set:
        store = runner.resolve_active_store(self._db_path())
        client, collection = runner._open_chroma_collection(
            store, runner.V2_FILES_CODE_COLLECTION
        )
        try:
            return set(collection.get().get("ids") or [])
        finally:
            runner._close_chroma_client(client)


class IncrementalGenerationTests(_IncrementalFixture):
    def test_same_repo_second_worktree_reuses_all_v2_embeddings(self):
        second_project = self._make_worktree_project("second-project")

        baseline = self._build_protocol_v2(self.project, WORKTREE_HASH)
        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("indexed"), 8, baseline)

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_batches = []

        def record_encode(texts, **kwargs):
            encoded_batches.append(list(texts))
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            result = self._build_protocol_v2(
                second_project,
                SECOND_WORKTREE_HASH,
            )

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("indexed"), 8, result)
        self.assertEqual(result.get("computed_embeddings"), 0, result)
        self.assertEqual(result.get("embedding_cache_hits"), 8, result)
        self.assertEqual(
            sum(len(batch) for batch in encoded_batches),
            0,
            f"the second identical worktree must not encode: {encoded_batches}",
        )
        self.assertEqual(counting.encode.call_count, 0)

    def test_same_repo_reuse_survives_a_fresh_runner_process(self):
        second_project = self._make_worktree_project("fresh-process-project")

        baseline = self._run_protocol_v2_subprocess(self.project, WORKTREE_HASH)
        reused = self._run_protocol_v2_subprocess(
            second_project,
            SECOND_WORKTREE_HASH,
        )

        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("computed_embeddings"), 8, baseline)
        self.assertTrue(reused.get("ok"), reused)
        self.assertEqual(reused.get("indexed"), 8, reused)
        self.assertEqual(
            reused.get("computed_embeddings"),
            0,
            "repo-scoped CAS reuse must survive one-shot runner process exit",
        )
        self.assertEqual(reused.get("embedding_cache_hits"), 8, reused)

    def test_twenty_worktrees_compute_only_unique_base_and_overlay_inputs(self):
        canonical_repo = self._make_canonical_repo()
        base_dir_resolver = getattr(runner, "resolve_file_index_v2_base_dir", None)
        overlay_dir_resolver = getattr(
            runner, "resolve_file_index_v2_overlay_dir", None
        )
        self.assertIsNotNone(
            base_dir_resolver,
            "Phase 71 requires physical canonical base generation resolution",
        )
        self.assertIsNotNone(
            overlay_dir_resolver,
            "Phase 71 requires physical worktree overlay generation resolution",
        )
        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_inputs = []

        def record_encode(texts, **kwargs):
            encoded_inputs.extend(texts)
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        computed = 0
        base_generation_ids = set()
        overlay_generation_ids = set()
        physical_base_dirs = set()
        physical_overlay_dirs = set()
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            for index in range(20):
                project = self._clone_canonical_worktree(
                    canonical_repo,
                    f"worktree-{index:02}",
                    overlay_index=index,
                )
                result = self._build_protocol_v2(
                    project,
                    f"{index + 1:016x}",
                )
                self.assertTrue(result.get("ok"), result)
                self.assertEqual(result.get("indexed"), 9, result)
                self.assertIsInstance(result.get("computed_embeddings"), int, result)
                computed += result["computed_embeddings"]
                self.assertEqual(result.get("base_document_count"), 8, result)
                self.assertEqual(result.get("overlay_document_count"), 1, result)
                self.assertIsInstance(result.get("base_generation_id"), str, result)
                self.assertIsInstance(result.get("overlay_generation_id"), str, result)
                base_generation_ids.add(result["base_generation_id"])
                overlay_generation_ids.add(result["overlay_generation_id"])
                base_dir = Path(
                    base_dir_resolver(
                        REPO_HASH,
                        result["base_generation_id"],
                        db_root=self.db_root,
                    )
                )
                overlay_dir = Path(
                    overlay_dir_resolver(
                        REPO_HASH,
                        f"{index + 1:016x}",
                        result["overlay_generation_id"],
                        db_root=self.db_root,
                    )
                )
                self.assertEqual(self._physical_collection_count(base_dir), 8)
                self.assertEqual(self._physical_collection_count(overlay_dir), 1)
                physical_base_dirs.add(base_dir.resolve())
                physical_overlay_dirs.add(overlay_dir.resolve())

            self.assertLessEqual(
                computed,
                8 + 20,
                "20 worktrees must compute the 8 shared inputs once and only "
                "one unique overlay input per worktree",
            )
            self.assertEqual(
                computed,
                len(encoded_inputs),
                "computed_embeddings must report actual model encode inputs",
            )
            self.assertEqual(
                len(encoded_inputs),
                28,
                "the empty repo-scoped CAS must populate each of the 28 exact "
                "unique model inputs once",
            )
            self.assertEqual(
                len(set(encoded_inputs)),
                28,
                "no exact model input may be recomputed across worktrees",
            )
            self.assertEqual(
                len(base_generation_ids),
                1,
                "all 20 worktrees must pin the same canonical base generation",
            )
            self.assertEqual(
                len(overlay_generation_ids),
                20,
                "each unique worktree source snapshot needs its own overlay generation",
            )
            self.assertEqual(
                len(physical_base_dirs),
                1,
                "the v2 store must contain one physical canonical base generation",
            )
            self.assertEqual(
                len(physical_overlay_dirs),
                20,
                "the v2 store must contain one physical 1-record overlay per worktree",
            )

            identical = self._clone_canonical_worktree(
                canonical_repo,
                "identical-overlay",
                overlay_index=0,
            )
            encoded_before = len(encoded_inputs)
            first_refresh = self._build_protocol_v2(
                identical,
                "0000000000000015",
            )
            self.assertTrue(first_refresh.get("ok"), first_refresh)
            self.assertEqual(first_refresh.get("indexed"), 9, first_refresh)
            self.assertEqual(first_refresh.get("base_document_count"), 8, first_refresh)
            self.assertEqual(
                first_refresh.get("overlay_document_count"), 1, first_refresh
            )
            self.assertIn(first_refresh.get("base_generation_id"), base_generation_ids)
            self.assertEqual(first_refresh.get("computed_embeddings"), 0, first_refresh)
            self.assertEqual(
                len(encoded_inputs),
                encoded_before,
                "a first refresh of an identical overlay must be a full CAS hit",
            )

            second_refresh = self._build_protocol_v2(
                identical,
                "0000000000000015",
            )
            self.assertTrue(second_refresh.get("ok"), second_refresh)
            self.assertEqual(second_refresh.get("indexed"), 9, second_refresh)
            self.assertEqual(
                second_refresh.get("computed_embeddings"), 0, second_refresh
            )
            self.assertEqual(
                len(encoded_inputs),
                encoded_before,
                "a same-worktree second refresh must not call model encode",
            )

    def test_different_repo_hashes_do_not_share_embedding_cas(self):
        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_inputs = []

        def record_encode(texts, **kwargs):
            encoded_inputs.extend(texts)
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            first = self._build_protocol_v2(
                self.project,
                WORKTREE_HASH,
                repo_hash=REPO_HASH,
            )
            first_encoded = len(encoded_inputs)
            second = self._build_protocol_v2(
                self.project,
                WORKTREE_HASH,
                repo_hash=SECOND_REPO_HASH,
            )
            second_encoded = len(encoded_inputs) - first_encoded

        self.assertTrue(first.get("ok"), first)
        self.assertTrue(second.get("ok"), second)
        self.assertEqual(first.get("computed_embeddings"), 8, first)
        self.assertEqual(second.get("computed_embeddings"), 8, second)
        self.assertEqual(first_encoded, 8)
        self.assertEqual(
            second_encoded,
            8,
            "an exact input cached under another repo hash must still be encoded",
        )

    def test_cas_key_builder_output_controls_lookup_and_population(self):
        key_builder = getattr(runner, "file_embedding_cas_key", None)
        self.assertIsNotNone(
            key_builder,
            "Phase 71 requires file_embedding_cas_key for exact model inputs",
        )
        second_project = self._make_worktree_project("isolated-cas-key-project")
        baseline = self._build_protocol_v2(self.project, WORKTREE_HASH)
        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("computed_embeddings"), 8, baseline)

        def isolate_cas_namespace(contract, model_input):
            identity = dict(key_builder(contract, model_input))
            identity["cas_key"] = hashlib.sha256(
                f"isolated-test:{identity['cas_key']}".encode("utf-8")
            ).hexdigest()
            return identity

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_inputs = []

        def record_encode(texts, **kwargs):
            encoded_inputs.extend(texts)
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        with mock.patch.object(
            runner,
            "file_embedding_cas_key",
            side_effect=isolate_cas_namespace,
        ) as isolated_key:
            with mock.patch.object(
                runner,
                "_get_embedding_model",
                return_value=counting,
            ):
                result = self._build_protocol_v2(
                    second_project,
                    SECOND_WORKTREE_HASH,
                )

        self.assertTrue(result.get("ok"), result)
        self.assertGreaterEqual(isolated_key.call_count, 8)
        self.assertEqual(result.get("computed_embeddings"), 8, result)
        self.assertEqual(
            len(encoded_inputs),
            8,
            "changing only the returned CAS key must force eight cold populates",
        )

    def test_path_only_rename_invalidates_exact_input_cas(self):
        baseline = self._build_protocol_v2(self.project, WORKTREE_HASH)
        self.assertTrue(baseline.get("ok"), baseline)
        original = self.src / "module_00.rs"
        renamed = self.src / "renamed_00.rs"
        original.rename(renamed)

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_inputs = []

        def record_encode(texts, **kwargs):
            encoded_inputs.extend(texts)
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            result = self._build_protocol_v2(self.project, WORKTREE_HASH)

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("indexed"), 8, result)
        self.assertEqual(result.get("computed_embeddings"), 1, result)
        self.assertEqual(len(encoded_inputs), 1)
        self.assertIn(
            "path: src/renamed_00.rs",
            encoded_inputs[0],
            "the renamed path must be part of the final passage input",
        )

    def test_descriptor_rejection_controls_artifact_and_cas_reuse(self):
        compatibility = getattr(runner, "file_index_compatibility", None)
        self.assertIsNotNone(
            compatibility,
            "Phase 71 requires pure file_index_compatibility descriptor checking",
        )
        baseline_descriptor = self._compatibility_descriptor()
        model_changed = copy.deepcopy(baseline_descriptor)
        model_changed["model_revision"] = "model-revision-b"
        document_changed = copy.deepcopy(baseline_descriptor)
        document_changed["document_contract"]["payload_builder_version"] = 2

        baseline_project = self._make_worktree_project("descriptor-baseline")
        model_project = self._make_worktree_project("descriptor-model-change")
        document_project = self._make_worktree_project("descriptor-document-change")
        baseline = self._build_protocol_v2(
            baseline_project,
            "0000000000000101",
            compatibility_descriptor=baseline_descriptor,
        )
        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("computed_embeddings"), 8, baseline)

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_inputs = []

        def record_encode(texts, **kwargs):
            encoded_inputs.extend(texts)
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        with mock.patch.object(
            runner,
            "file_index_compatibility",
            wraps=compatibility,
        ) as checked_compatibility:
            with mock.patch.object(
                runner,
                "_get_embedding_model",
                return_value=counting,
            ):
                changed_model_result = self._build_protocol_v2(
                    model_project,
                    "0000000000000102",
                    compatibility_descriptor=model_changed,
                )
                changed_document_result = self._build_protocol_v2(
                    document_project,
                    "0000000000000103",
                    compatibility_descriptor=document_changed,
                )

        self.assertGreater(
            checked_compatibility.call_count,
            0,
            "artifact selection must use file_index_compatibility",
        )
        self.assertTrue(changed_model_result.get("ok"), changed_model_result)
        self.assertGreaterEqual(
            changed_model_result.get("compatibility_rejections", 0),
            1,
            changed_model_result,
        )
        self.assertNotEqual(
            changed_model_result.get("base_generation_id"),
            baseline.get("base_generation_id"),
        )
        self.assertEqual(changed_model_result.get("computed_embeddings"), 8)

        self.assertTrue(changed_document_result.get("ok"), changed_document_result)
        self.assertGreaterEqual(
            changed_document_result.get("compatibility_rejections", 0),
            1,
            changed_document_result,
        )
        self.assertNotEqual(
            changed_document_result.get("base_generation_id"),
            baseline.get("base_generation_id"),
        )
        self.assertEqual(
            changed_document_result.get("computed_embeddings"),
            0,
            "a document contract mismatch rejects the artifact but exact-input "
            "vectors remain compatible with the unchanged embedding contract",
        )
        self.assertEqual(
            len(encoded_inputs),
            8,
            "only the model contract change may invalidate exact-input CAS entries",
        )

    def test_rebuild_reuses_unchanged_embeddings_and_encodes_only_changes(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        self.assertEqual(baseline.get("indexed"), 8, baseline)

        # 2 changed, 1 added, 1 deleted.
        self._write_doc(0, "//! module 0 rewritten\nfn feature_0_v2() {}\n")
        self._write_doc(1, "//! module 1 rewritten\nfn feature_1_v2() {}\n")
        added = self._write_doc(90, "//! module 90 added\nfn feature_90() {}\n")
        removed = self.src / "module_07.rs"
        removed.unlink()

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_batches = []

        def record_encode(texts):
            encoded_batches.append(list(texts))
            return real_model.encode(texts)

        counting.encode.side_effect = record_encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            result = self._build()

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(
            result.get("indexed"), 8, f"8 - 1 deleted + 1 added = 8: {result}"
        )
        self.assertEqual(
            result.get("newly_embedded"),
            3,
            f"only the 2 changed + 1 added records may be encoded: {result}",
        )
        self.assertEqual(
            result.get("reused_embeddings"),
            5,
            f"unchanged records must reuse previous embeddings: {result}",
        )
        encoded_count = sum(len(batch) for batch in encoded_batches)
        self.assertEqual(
            encoded_count,
            3,
            f"model must only encode changed documents, encoded: {encoded_batches}",
        )

        ids = self._store_ids()
        self.assertIn("src/module_90.rs", ids, ids)
        self.assertNotIn(
            "src/module_07.rs",
            ids,
            "deleted records must be excluded from the new generation",
        )

    def test_unchanged_source_rebuild_encodes_nothing(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        counting.encode.side_effect = real_model.encode
        with mock.patch.object(runner, "_get_embedding_model", return_value=counting):
            result = self._build()

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("newly_embedded"), 0, result)
        self.assertEqual(result.get("reused_embeddings"), 8, result)
        self.assertEqual(
            counting.encode.call_count,
            0,
            "an unchanged corpus must not re-encode anything (FR-391)",
        )

    def test_source_change_during_build_aborts_publish_and_keeps_active(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)

        # Change one record so the rebuild embeds something, and mutate a
        # DIFFERENT source file during the embedding phase: the late
        # revalidation (FR-392 / AS-10) must detect the drift after taking
        # the publish boundary and refuse to publish the stale generation.
        self._write_doc(2, "//! module 2 rewritten\nfn feature_2_v2() {}\n")
        real_model = runner._FakeEmbeddingModel()
        mutating = mock.MagicMock()
        state = {"mutated": False}

        def mutate_once(texts):
            if not state["mutated"]:
                state["mutated"] = True
                self._write_doc(
                    5, "//! module 5 mutated mid-build\nfn feature_5_v3() {}\n"
                )
            return real_model.encode(texts)

        mutating.encode.side_effect = mutate_once
        with mock.patch.object(runner, "_get_embedding_model", return_value=mutating):
            result = self._build()

        self.assertFalse(
            result.get("ok"),
            f"a stale generation must not be published silently: {result}",
        )
        self.assertEqual(result.get("error_code"), "SOURCE_CHANGED", result)
        self.assertTrue(result.get("retryable"), result)

        # The active pointer still serves the previous healthy generation.
        status = runner._scope_status_v2(
            REPO_HASH, WORKTREE_HASH, "files", db_root=self.db_root
        )
        self.assertEqual(status["document_count"], 8, status)

class IncrementalModeRoutingTests(_IncrementalFixture):
    """PR #3301 review: `mode=\"incremental\"` must update the store that
    readers actually resolve (the active generation), not the legacy
    `db_path` that full mode has already migrated away from."""

    def _build_incremental(self) -> dict:
        return runner.action_index_files_v2(
            project_root=str(self.project),
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            mode="incremental",
            db_root=self.db_root,
            scope="files",
        )

    def test_incremental_mode_updates_the_active_generation(self):
        baseline = self._build()
        self.assertTrue(baseline.get("ok"), baseline)
        db_path = self._db_path()
        self.assertTrue(
            runner.active_pointer_path(db_path).is_file(),
            "full mode must publish an active generation first",
        )

        self._write_doc(90, "//! module 90 added\nfn feature_90() {}\n")
        (self.src / "module_07.rs").unlink()
        result = self._build_incremental()
        self.assertTrue(result.get("ok"), result)

        ids = self._store_ids()
        self.assertIn(
            "src/module_90.rs",
            ids,
            "incremental additions must reach the active generation readers use",
        )
        self.assertNotIn(
            "src/module_07.rs",
            ids,
            "incremental deletions must reach the active generation readers use",
        )
        self.assertFalse(
            (db_path / "chroma.sqlite3").exists(),
            "incremental mode must not resurrect the migrated legacy store",
        )


class PublishVerificationTests(unittest.TestCase):
    """PR #3301 review (Critical): a staging build whose verification read
    fails must never replace the healthy active generation."""

    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        self.db_root = self.base / "index"
        coord = self.base / "coordinator"
        coord.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(coord),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def test_unverifiable_staging_is_not_published(self):
        project = self.base / "project"
        project.mkdir()
        baseline = runner.action_index_board_v2(
            repo_hash=REPO_HASH,
            project_root=str(project),
            mode="full",
            db_root=self.db_root,
        )
        self.assertTrue(baseline.get("ok"), baseline)
        db_path = runner.resolve_db_path(REPO_HASH, None, "board", db_root=self.db_root)
        pointer_before = runner._read_active_pointer(db_path)
        self.assertIsNotNone(pointer_before)

        with mock.patch.object(
            runner, "_open_chroma_collection", side_effect=RuntimeError("cannot open")
        ):
            result = runner.action_index_board_v2(
                repo_hash=REPO_HASH,
                project_root=str(project),
                mode="full",
                db_root=self.db_root,
            )

        self.assertFalse(
            result.get("ok"),
            f"an unverifiable staging build must not publish silently: {result}",
        )
        self.assertEqual(result.get("error_code"), "BUILD_VERIFY_FAILED", result)
        pointer_after = runner._read_active_pointer(db_path)
        self.assertEqual(
            pointer_before,
            pointer_after,
            "the healthy active generation must remain in place",
        )



if __name__ == "__main__":
    unittest.main()
