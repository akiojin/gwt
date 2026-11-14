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
import contextlib
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import types
import unittest
from pathlib import Path
from typing import Optional
from unittest import mock

import chroma_index_runner as runner
from tests.git_fixture import git_fixture_env

REPO_HASH = "abc1234567890def"
SECOND_REPO_HASH = "fedcba0987654321"
WORKTREE_HASH = "111122223333ffff"
SECOND_WORKTREE_HASH = "aaaabbbbccccdddd"


class FileVectorVerificationTests(unittest.TestCase):
    def test_native_vector_comparison_preserves_scalar_boundary_without_scalar_loop(self):
        import numpy as np

        stored = np.array([0.0, 1.0], dtype=np.float32)
        with mock.patch.object(runner.math, "isclose", wraps=runner.math.isclose) as scalar:
            self.assertTrue(runner._chroma_vector_matches_cas(stored, [1e-6, 1.000001]))
            self.assertFalse(runner._chroma_vector_matches_cas(
                stored, [float(np.nextafter(1e-6, np.inf)), 1.0]
            ))
            self.assertFalse(runner._chroma_vector_matches_cas(
                np.array([1.0], dtype=np.float32), [float(np.nextafter(1.000001, np.inf))]
            ))
            self.assertFalse(runner._chroma_vector_matches_cas(np.array([np.nan]), [0.0]))
            self.assertFalse(runner._chroma_vector_matches_cas(np.array([np.inf]), [float("inf")]))
            self.assertFalse(runner._chroma_vector_matches_cas(np.array([0.0]), [float("nan")]))
            self.assertFalse(runner._chroma_vector_matches_cas(stored, [0.0]))
            scalar.assert_not_called()
            # Generic conversion remains on the original scalar fallback.
            self.assertTrue(runner._chroma_vector_matches_cas(["1.0"], [1.0]))
            self.assertTrue(runner._chroma_vector_matches_cas(np.array([1.0]), ["1.0"]))
            self.assertEqual(scalar.call_count, 2)

    def test_staging_verification_uses_the_fresh_cas_checksum_without_rehashing(self):
        compatibility = dict(runner.default_file_index_compatibility(), dimension=2)
        descriptor = {"kind": "base", "compatibility": compatibility}
        vector = [0.25, -0.5]
        checksum = runner._vector_checksum(vector)
        metadata = {"path": "src/unit.rs"}
        record = {"path": "src/unit.rs", "bucket": "code", "document": "unit", "metadata": metadata}
        identity = {"cas_key": "a" * 64, "input_digest": "b" * 64}
        entry = {"path": record["path"], "scope": "files", "dimension": 2,
                 "vector_checksum": checksum, "payload_digest": hashlib.sha256(b"unit").hexdigest(),
                 "metadata_digest": runner._sha256_json(metadata)}
        collection = mock.Mock()
        collection.get.return_value = {"ids": [runner._file_record_id(entry)], "documents": ["unit"],
                                       "metadatas": [metadata], "embeddings": [vector]}
        cas_root = Path("file-index-v2/cas")
        with mock.patch.object(runner, "_artifact_pair_metadata_is_verified", return_value=True), \
                mock.patch.object(runner, "_open_file_index_v2_collection", return_value=(mock.Mock(), collection)), \
                mock.patch.object(runner, "_read_cas_vector", return_value=vector) as read_cas, \
                mock.patch.object(runner, "_vector_checksum", side_effect=AssertionError("fresh CAS read already hashed")):
            self.assertTrue(runner._staged_artifact_vectors_are_verified(
                Path("file-index-v2/bases/staging"), [record], [identity], [entry], descriptor, cas_root
            ))
        read_cas.assert_called_once_with(
            cas_root, identity, runner._embedding_contract(compatibility), 2,
            expected_manifest_checksum=checksum,
        )


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
        try:
            completed = subprocess.run(
                ["git", *args],
                check=False,
                capture_output=True,
                text=True,
                env=git_fixture_env(),
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

    def _artifact_collection_count(
        self, artifact_dir: Path, collection_name: str = runner.V2_FILES_CODE_COLLECTION
    ) -> int:
        descriptor = json.loads(
            (artifact_dir / "descriptor.json").read_text(encoding="utf-8")
        )
        store = runner._file_artifact_store(artifact_dir, descriptor)
        self.assertTrue((store / "chroma.sqlite3").is_file())
        client, collection = runner._open_file_index_v2_collection(
            store, collection_name
        )
        try:
            if runner._uses_file_record_pool(descriptor):
                entries = json.loads(
                    (artifact_dir / "manifest.json").read_text(encoding="utf-8")
                )["entries"]
                scope = "files" if collection_name == runner.V2_FILES_CODE_COLLECTION else "files-docs"
                ids = runner._file_member_ids(entries, scope)
                stored_ids = collection.get(ids=ids, include=[])["ids"] if ids else []
                self.assertEqual(set(stored_ids), set(ids))
                return len(stored_ids)
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
    def test_file_record_pool_pruning_retains_members_and_respects_reader_pin(self):
        result = self._build_protocol_v2(self.project, WORKTREE_HASH)
        self.assertTrue(result.get("ok"), result)
        v2_root = runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root)
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            REPO_HASH, WORKTREE_HASH, result["overlay_generation_id"], db_root=self.db_root
        )
        descriptor = runner._read_artifact_descriptor(overlay_dir)
        store = runner._file_artifact_store(overlay_dir, descriptor)
        client, collection = runner._open_file_index_v2_collection(store, runner.V2_FILES_DOCS_COLLECTION)
        try:
            collection.upsert(ids=["orphan"], documents=["unused"],
                              embeddings=[[1.0] * runner._FakeEmbeddingModel.DIM],
                              metadatas=[{"path": "unused.rs"}])
        finally:
            runner._close_chroma_client(client)
        future_artifact = v2_root / "bases" / "future-artifact"
        future_artifact.mkdir(parents=True)
        future_descriptor = copy.deepcopy(descriptor)
        future_descriptor["compatibility"]["writer_protocol"] = "future-writer"
        (future_artifact / "descriptor.json").write_text(json.dumps(future_descriptor))
        self.assertEqual(runner._prune_file_record_pool(v2_root), 0)
        (future_artifact / "descriptor.json").unlink()
        future_artifact.rmdir()
        with runner._file_index_v2_pin(REPO_HASH, WORKTREE_HASH, "reader", self.db_root):
            self.assertEqual(runner._prune_file_record_pool(v2_root), 0)
        self.assertEqual(runner._prune_file_record_pool(v2_root), 1)
        client, collection = runner._open_file_index_v2_collection(store, runner.V2_FILES_CODE_COLLECTION)
        try:
            self.assertEqual(collection.count(), 8)
        finally:
            runner._close_chroma_client(client)

    def test_canonical_refresh_reuses_records_across_successive_file_changes(self):
        project = self._make_canonical_repo()
        first = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(first.get("ok"), first)
        original_make = runner._make_file_index_v2_collection
        original_open = runner._open_file_index_v2_collection
        written_paths = []
        vector_read_ids = []

        def observed_collection(factory, *args, **kwargs):
            client, collection = factory(*args, **kwargs)
            original_upsert = collection.upsert

            def observed_upsert(*args, **kwargs):
                written_paths.extend(meta["path"] for meta in kwargs["metadatas"])
                return original_upsert(*args, **kwargs)

            def observed_get(*args, **kwargs):
                if "embeddings" in (kwargs.get("include") or []):
                    self.assertIsNotNone(kwargs.get("ids"), "vector reads must be bounded by exact IDs")
                    vector_read_ids.extend(kwargs["ids"])
                return collection.get(*args, **kwargs)

            return client, mock.Mock(wraps=collection, upsert=observed_upsert, get=observed_get)

        for index in (0, 1):
            changed_path = f"src/module_{index:02}.rs"
            path = project / changed_path
            path.write_text(f"//! updated module {index}\nfn updated_{index}() {{}}\n", encoding="utf-8")
            self._run_git("-C", str(project), "add", "src")
            self._run_git(
                "-C", str(project), "-c", "user.name=gwt tests", "-c",
                "user.email=gwt-tests@example.invalid", "commit", "--quiet",
                "-m", f"change module {index}",
            )
            written_paths.clear()
            vector_read_ids.clear()
            with contextlib.ExitStack() as patches:
                patches.enter_context(mock.patch.object(
                    runner, "_make_file_index_v2_collection",
                    side_effect=lambda *args, **kwargs: observed_collection(original_make, *args, **kwargs),
                ))
                patches.enter_context(mock.patch.object(
                    runner, "_open_file_index_v2_collection",
                    side_effect=lambda *args, **kwargs: observed_collection(original_open, *args, **kwargs),
                ))
                resolve = patches.enter_context(mock.patch.object(
                    runner, "_resolve_record_vector_batch", wraps=runner._resolve_record_vector_batch,
                ))
                cas_read = patches.enter_context(mock.patch.object(
                    runner, "_read_cas_vector", wraps=runner._read_cas_vector,
                ))
                parse = patches.enter_context(mock.patch.object(
                    runner, "_record_from_bytes", wraps=runner._record_from_bytes,
                ))
                result = self._build_protocol_v2(project, WORKTREE_HASH)
            self.assertTrue(result.get("ok"), result)
            self.assertEqual(written_paths, [changed_path])
            resolved_paths = [record["path"] for call in resolve.call_args_list for record in call.args[0]]
            self.assertEqual(resolved_paths, [changed_path], "unchanged immutable records must bypass CAS resolution")
            self.assertEqual([call.args[0] for call in parse.call_args_list], [changed_path])
            base_dir = runner.resolve_file_index_v2_base_dir(
                REPO_HASH, result["base_generation_id"], db_root=self.db_root,
            )
            entries = json.loads((base_dir / "manifest.json").read_text(encoding="utf-8"))["entries"]
            changed_entry = next(entry for entry in entries if entry["path"] == changed_path)
            self.assertEqual({call.args[1]["cas_key"] for call in cas_read.call_args_list}, {changed_entry["cas_key"]})
            self.assertEqual(set(vector_read_ids), {runner._file_record_id(changed_entry)})
            self.assertEqual(result.get("computed_embeddings"), 1, result)
            self.assertEqual(result.get("embedding_cache_hits"), 7, result)

        overlay_path = "src/module_02.rs"
        (project / overlay_path).write_text("//! uncommitted overlay\nfn overlay() {}\n", encoding="utf-8")
        with mock.patch.object(runner, "_git_cat_file_batch", side_effect=AssertionError("unchanged canonical blobs must reuse verified payloads")), \
                mock.patch.object(runner, "_record_from_bytes", wraps=runner._record_from_bytes) as parse:
            overlay = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(overlay.get("ok"), overlay)
        self.assertEqual(overlay["base_generation_id"], result["base_generation_id"])
        self.assertEqual({call.args[0] for call in parse.call_args_list}, {overlay_path})
        self.assertEqual(overlay.get("computed_embeddings"), 1, overlay)

        first_path, second_path = project / "src/first.rs", project / "src/second.rs"
        first_path.write_text("fn first() {}\n", encoding="utf-8")
        first_overlay = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(first_overlay.get("ok"), first_overlay)
        first_path.unlink()
        second_path.write_text("fn second() {}\n", encoding="utf-8")
        second_overlay = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(second_overlay.get("ok"), second_overlay)
        second_path.unlink()
        first_path.write_text("fn first() {}\n", encoding="utf-8")
        with mock.patch.object(runner, "_resolve_record_vector_batch", side_effect=AssertionError("healthy retained artifacts must remain warm")):
            restored = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(restored.get("ok"), restored)
        self.assertEqual(restored["overlay_generation_id"], first_overlay["overlay_generation_id"])

    def test_new_base_repairs_unproven_members_without_reading_unused_cas(self):
        project = self._make_canonical_repo()
        first = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(first.get("ok"), first)
        base_dir = runner.resolve_file_index_v2_base_dir(
            REPO_HASH, first["base_generation_id"], db_root=self.db_root,
        )
        descriptor = runner._read_artifact_descriptor(base_dir)
        entries = {entry["path"]: entry for entry in json.loads(
            (base_dir / "manifest.json").read_text(encoding="utf-8"),
        )["entries"]}
        store = runner._file_artifact_store(base_dir, descriptor)
        repaired_paths = [f"src/module_{index:02}.rs" for index in (4, 5, 6)]
        repair_ids = [runner._file_record_id(entries[path]) for path in repaired_paths]
        client, collection = runner._open_file_index_v2_collection(store, runner.V2_FILES_CODE_COLLECTION)
        try:
            expected = collection.get(ids=repair_ids, include=["documents", "metadatas", "embeddings"])
            vectors = dict(zip(expected["ids"], expected["embeddings"]))
            collection.update(ids=[repair_ids[0]], metadatas=[{"path": "incorrect.rs"}])
            collection.delete(ids=[repair_ids[1]])
            collection.update(ids=[repair_ids[2]], documents=["not the source payload"],
                              embeddings=[vectors[repair_ids[2]]])
        finally:
            runner._close_chroma_client(client)

        cas_root = runner.resolve_file_index_v2_root(REPO_HASH, db_root=self.db_root) / "cas"
        unused_key = entries["src/module_07.rs"]["cas_key"]
        unused_cas = runner._cas_entry_path(cas_root, unused_key)
        unused_cas.write_bytes(b"invalid unused CAS")
        changed_path = "src/module_00.rs"
        path = project / changed_path
        path.write_text("//! updated module 0\nfn updated_0() {}\n", encoding="utf-8")
        changed_record = runner._record_from_bytes(changed_path, path.read_bytes())
        identities, _ = runner._plan_file_artifact([changed_record], descriptor["compatibility"])
        needed_cas = runner._cas_entry_path(cas_root, identities[0]["cas_key"])
        needed_cas.parent.mkdir(parents=True, exist_ok=True)
        needed_cas.write_bytes(b"invalid needed CAS")
        self._run_git("-C", str(project), "add", "src")
        self._run_git("-C", str(project), "-c", "user.name=gwt tests", "-c",
                      "user.email=gwt-tests@example.invalid", "commit", "--quiet", "-m", "change module 0")

        with mock.patch.object(runner, "_resolve_record_vector_batch", wraps=runner._resolve_record_vector_batch) as resolve, \
                mock.patch.object(runner, "_read_cas_vector", wraps=runner._read_cas_vector) as cas_read, \
                mock.patch.object(runner, "_record_from_bytes", wraps=runner._record_from_bytes) as parse:
            result = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("computed_embeddings"), 1, result)
        self.assertEqual(result.get("embedding_cache_hits"), 7, result)
        expected_paths = {changed_path, *repaired_paths}
        self.assertEqual({record["path"] for call in resolve.call_args_list for record in call.args[0]}, expected_paths)
        self.assertEqual({call.args[0] for call in parse.call_args_list}, expected_paths,
                         "missing or digest-invalid actual payloads must return to source parsing")
        self.assertNotIn(unused_key, [call.args[1]["cas_key"] for call in cas_read.call_args_list])
        self.assertEqual(unused_cas.read_bytes(), b"invalid unused CAS")
        self.assertNotEqual(needed_cas.read_bytes(), b"invalid needed CAS")
        client, collection = runner._open_file_index_v2_collection(store, runner.V2_FILES_CODE_COLLECTION)
        try:
            actual = collection.get(ids=repair_ids, include=["documents", "metadatas"])
            self.assertEqual(
                dict(zip(actual["ids"], zip(actual["documents"], actual["metadatas"]))),
                dict(zip(expected["ids"], zip(expected["documents"], expected["metadatas"]))),
                "missing IDs and each invalid payload field must be repaired",
            )
            collection.update(ids=[repair_ids[0]], documents=["same-tree payload corruption"],
                              embeddings=[vectors[repair_ids[0]]])
        finally:
            runner._close_chroma_client(client)

        previous_base = result["base_generation_id"]
        repair_base = runner.resolve_file_index_v2_base_dir(
            REPO_HASH, previous_base, db_root=self.db_root,
        )
        head_path = repair_base.parents[1] / "worktrees" / WORKTREE_HASH / "head.json"
        retained = {path: path.read_bytes() for path in (
            repair_base / "descriptor.json", repair_base / "manifest.json", head_path,
        )}
        # Force a real encode so background repair yields after this record.
        runner._cas_entry_path(cas_root, entries[repaired_paths[0]]["cas_key"]).unlink()
        with mock.patch.object(runner, "_pending_higher_priority", return_value=True):
            yielded = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(yielded.get("yielded"), yielded)
        for path, contents in retained.items():
            self.assertTrue(path.is_file(), f"interrupted repair removed {path}")
            self.assertEqual(path.read_bytes(), contents)
        # Recreate the corruption to check the ordinary repair path on resume.
        client, collection = runner._open_file_index_v2_collection(store, runner.V2_FILES_CODE_COLLECTION)
        try:
            collection.update(ids=[repair_ids[0]], documents=["same-tree payload corruption"],
                              embeddings=[vectors[repair_ids[0]]])
        finally:
            runner._close_chroma_client(client)
        with mock.patch.object(runner, "_resolve_record_vector_batch", wraps=runner._resolve_record_vector_batch) as resolve, \
                mock.patch.object(runner, "_record_from_bytes", wraps=runner._record_from_bytes) as parse:
            result = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result["base_generation_id"], previous_base)
        self.assertEqual({call.args[0] for call in parse.call_args_list}, {repaired_paths[0]})
        self.assertEqual({record["path"] for call in resolve.call_args_list for record in call.args[0]}, {repaired_paths[0]})
        self.assertEqual(unused_cas.read_bytes(), b"invalid unused CAS")
        client, collection = runner._open_file_index_v2_collection(store, runner.V2_FILES_CODE_COLLECTION)
        try:
            self.assertEqual(collection.get(ids=[repair_ids[0]], include=["documents"])["documents"],
                             [expected["documents"][expected["ids"].index(repair_ids[0])]])
            collection.update(ids=[repair_ids[0]], documents=["repair interrupted at publication"],
                              embeddings=[vectors[repair_ids[0]]])
        finally:
            runner._close_chroma_client(client)
        retained = {path: path.read_bytes() for path in retained}
        with mock.patch.object(runner, "_durably_replace_file_index_v2_directory",
                               side_effect=RuntimeError("publication interrupted")):
            with self.assertRaisesRegex(RuntimeError, "publication interrupted"):
                self._build_protocol_v2(project, WORKTREE_HASH)
        for path, contents in retained.items():
            self.assertTrue(path.is_file(), f"failed publication removed {path}")
            self.assertEqual(path.read_bytes(), contents)
        resumed = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(resumed.get("ok"), resumed)
        self.assertEqual(resumed["base_generation_id"], previous_base)

    def test_new_base_recreates_a_missing_native_collection(self):
        project = self._make_canonical_repo()
        first = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(first.get("ok"), first)
        base_dir = runner.resolve_file_index_v2_base_dir(
            REPO_HASH, first["base_generation_id"], db_root=self.db_root,
        )
        store = runner._file_artifact_store(base_dir, runner._read_artifact_descriptor(base_dir))
        client, _ = runner._open_file_index_v2_collection(store, runner.V2_FILES_CODE_COLLECTION)
        try:
            client.delete_collection(runner.V2_FILES_CODE_COLLECTION)
        finally:
            runner._close_chroma_client(client)
        self.assertTrue((store / "chroma.sqlite3").is_file())
        (project / "src/module_00.rs").write_text("//! updated module 0\nfn updated_0() {}\n", encoding="utf-8")
        self._run_git("-C", str(project), "add", "src")
        self._run_git("-C", str(project), "-c", "user.name=gwt tests", "-c",
                      "user.email=gwt-tests@example.invalid", "commit", "--quiet", "-m", "change module 0")
        result = self._build_protocol_v2(project, WORKTREE_HASH)
        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("computed_embeddings"), 1, result)
        self.assertEqual(result.get("embedding_cache_hits"), 7, result)
        rebuilt_base = runner.resolve_file_index_v2_base_dir(
            REPO_HASH, result["base_generation_id"], db_root=self.db_root,
        )
        self.assertEqual(self._artifact_collection_count(rebuilt_base), 8)

    def test_file_index_v2_collection_creation_failure_closes_client(self):
        for operation, client_method in (
            (runner._make_file_index_v2_collection, "get_or_create_collection"),
            (runner._open_file_index_v2_collection, "get_collection"),
        ):
            with self.subTest(operation=operation.__name__):
                client = mock.MagicMock()
                getattr(client, client_method).side_effect = RuntimeError("store failure")
                chromadb = types.ModuleType("chromadb")
                chromadb.PersistentClient = mock.Mock(return_value=client)
                with mock.patch.dict(sys.modules, {"chromadb": chromadb}):
                    with self.assertRaisesRegex(RuntimeError, "store failure"):
                        operation(self.db_root / "failed-store", "files_code")
                client.close.assert_called_once_with()

    def test_large_v2_corpus_uses_bounded_batches_and_metadata_only_warm_reuse(self):
        large_project = self.base / "large-project"
        large_src = large_project / "src"
        large_src.mkdir(parents=True)
        for index in range(130):
            (large_src / f"module_{index:03}.rs").write_text(
                f"//! large module {index}\nfn feature_{index}() {{}}\n",
                encoding="utf-8",
            )

        real_model = runner._FakeEmbeddingModel()
        encoded_batch_sizes = []

        def bounded_encode(texts, **kwargs):
            encoded_batch_sizes.append(len(texts))
            return real_model.encode(texts, **kwargs)

        model = mock.MagicMock()
        model.encode.side_effect = bounded_encode
        original_open = runner._open_file_index_v2_collection
        embedding_get_batch_sizes = []

        class ObservedCollection:
            def __init__(self, collection):
                self._collection = collection

            def __getattr__(self, name):
                return getattr(self._collection, name)

            def get(self, *args, **kwargs):
                include = kwargs.get("include") or []
                if "embeddings" in include:
                    ids = kwargs.get("ids")
                    embedding_get_batch_sizes.append(None if ids is None else len(ids))
                return self._collection.get(*args, **kwargs)

        def observed_open(*args, **kwargs):
            client, collection = original_open(*args, **kwargs)
            return client, ObservedCollection(collection)

        with mock.patch.object(runner, "_get_embedding_model", return_value=model):
            with mock.patch.object(
                runner,
                "_open_file_index_v2_collection",
                side_effect=observed_open,
            ):
                cold = self._build_protocol_v2(
                    large_project,
                    "0000000000000401",
                    repo_hash="abc1234567890401",
                )

        self.assertTrue(cold.get("ok"), cold)
        self.assertEqual(cold.get("requested_embeddings"), 130, cold)
        self.assertEqual(cold.get("computed_embeddings"), 130, cold)
        self.assertEqual(cold.get("embedding_cache_hits"), 0, cold)
        self.assertTrue(encoded_batch_sizes, cold)
        self.assertLessEqual(max(encoded_batch_sizes), 16, encoded_batch_sizes)
        self.assertEqual(sum(encoded_batch_sizes), 130, encoded_batch_sizes)
        self.assertTrue(embedding_get_batch_sizes, cold)
        self.assertNotIn(None, embedding_get_batch_sizes)
        self.assertLessEqual(max(embedding_get_batch_sizes), 16)

        membership_get_batch_sizes = []
        live_read_clients = {}
        readonly_pair_overlaps = []
        original_close = runner._close_chroma_client

        class MembershipOnlyCollection:
            def __init__(self, collection):
                self._collection = collection

            def __getattr__(self, name):
                return getattr(self._collection, name)

            def get(self, *args, **kwargs):
                self_ids = kwargs.get("ids")
                if self_ids is None or kwargs.get("include") != []:
                    raise AssertionError("warm verified reuse may only check member IDs")
                membership_get_batch_sizes.append(len(self_ids))
                return self._collection.get(*args, **kwargs)

        def count_only_open(*args, **kwargs):
            client, collection = original_open(*args, **kwargs)
            store, collection_name = args[:2]
            if collection_name == runner.V2_FILES_DOCS_COLLECTION:
                readonly_pair_overlaps.append(any(
                    previous == (store, runner.V2_FILES_CODE_COLLECTION)
                    for previous in live_read_clients.values()
                ))
            live_read_clients[id(client)] = (store, collection_name)
            return client, MembershipOnlyCollection(collection)

        def close_read_client(client):
            live_read_clients.pop(id(client), None)
            return original_close(client)

        with mock.patch.object(
            runner,
            "_read_cas_vector",
            side_effect=AssertionError("warm verified reuse must not read CAS vectors"),
        ) as cas_read:
            with mock.patch.object(
                runner,
                "_get_embedding_model",
                side_effect=AssertionError("warm verified reuse must not load the model"),
            ) as model_load:
                with (
                    mock.patch.object(runner, "_open_file_index_v2_collection", side_effect=count_only_open),
                    mock.patch.object(runner, "_close_chroma_client", side_effect=close_read_client),
                ):
                    warm = self._build_protocol_v2(
                        large_project,
                        "0000000000000401",
                        repo_hash="abc1234567890401",
                    )

        self.assertTrue(warm.get("ok"), warm)
        self.assertEqual(warm.get("base_generation_id"), cold["base_generation_id"])
        self.assertEqual(
            warm.get("overlay_generation_id"), cold["overlay_generation_id"]
        )
        self.assertEqual(warm.get("requested_embeddings"), 130, warm)
        self.assertEqual(warm.get("computed_embeddings"), 0, warm)
        self.assertEqual(warm.get("embedding_cache_hits"), 130, warm)
        cas_read.assert_not_called()
        model_load.assert_not_called()
        self.assertIn(128, membership_get_batch_sizes)
        self.assertLessEqual(max(membership_get_batch_sizes), 128)
        self.assertTrue(readonly_pair_overlaps)
        self.assertTrue(all(readonly_pair_overlaps), readonly_pair_overlaps)
        self.assertEqual(live_read_clients, {}, "read-only clients must close at the helper boundary")

        from chromadb.api.models.Collection import Collection
        get_records = Collection.get
        v2_root = runner.resolve_file_index_v2_root("abc1234567890401", db_root=self.db_root)
        with mock.patch.object(Collection, "get", autospec=True, side_effect=get_records) as gc_read:
            self.assertEqual(runner._prune_file_record_pool(v2_root), 0)
        self.assertEqual([call.kwargs for call in gc_read.call_args_list], [
            {"limit": 128, "offset": 0, "include": []},
            {"limit": 128, "offset": 128, "include": []},
        ])

    def test_ignore_matcher_reuses_identical_inputs_and_observes_fresh_rules(self):
        self._run_git("init", "--quiet", str(self.project))
        ignore = self.project / ".gitignore"
        ignore.write_text(f"unmatched-{self.base.name}\n*.rs\n", encoding="utf-8")
        patterns = [pattern for base, raw in runner.load_project_ignore_patterns(self.project)
                    if (pattern := runner._pattern_to_regex(raw, base)) is not None]
        relative_path = "src/module_00.rs"
        searches = []

        def record_search(frame, event, function):
            if (event == "c_call" and getattr(function, "__name__", None) == "search"
                    and isinstance(getattr(function, "__self__", None), runner.re.Pattern)):
                searches.append(function.__self__)

        previous_profiler = sys.getprofile()
        try:
            sys.setprofile(record_search)
            first = runner.should_ignore(relative_path, patterns)
            repeated = runner.should_ignore(relative_path, list(patterns))
        finally:
            sys.setprofile(previous_profiler)
        self.assertTrue(first)
        self.assertTrue(repeated)
        self.assertEqual(searches, patterns[:2], "only the first identical check may search through its first match")

        ignore.write_text("*.md\n", encoding="utf-8")
        changed = [pattern for base, raw in runner.load_project_ignore_patterns(self.project)
                   if (pattern := runner._pattern_to_regex(raw, base)) is not None]
        self.assertFalse(runner.should_ignore(relative_path, changed))
        self.assertTrue(runner.should_ignore(relative_path, [runner.re.compile(r"\.RS$", runner.re.IGNORECASE)]))
        self.assertFalse(runner.should_ignore(relative_path, [runner.re.compile(r"\.RS$")]))

    def test_v2_action_rejects_bad_arguments_before_source_or_store_access(self):
        invalid_descriptor = self._compatibility_descriptor()
        invalid_descriptor.pop("path_policy_hash")
        cases = (
            {"repo_hash": "../unsafe"},
            {"worktree_hash": "../unsafe"},
            {"project_root": str(self.base / "missing-project")},
            {"scope": "issues"},
            {"compatibility_descriptor": {}},
            {"compatibility_descriptor": invalid_descriptor},
        )

        for overrides in cases:
            arguments = {
                "project_root": str(self.project),
                "repo_hash": "abc1234567890402",
                "worktree_hash": "0000000000000402",
                "mode": "full",
                "db_root": self.db_root,
                "scope": "files",
                "file_index_protocol": "v2",
            }
            arguments.update(overrides)
            with self.subTest(overrides=overrides):
                with mock.patch.object(
                    runner, "_visible_file_records", return_value=[]
                ) as scan:
                    with mock.patch.object(
                        runner, "_canonical_git_tree", return_value=None
                    ) as git_probe:
                        with mock.patch.object(
                            runner, "_materialize_file_artifact_pair"
                        ) as materialize:
                            with self.assertRaises(ValueError):
                                runner.action_index_files_v2(**arguments)
                scan.assert_not_called()
                git_probe.assert_not_called()
                materialize.assert_not_called()

    def test_canonical_capture_consolidates_git_probes_without_changing_the_pinned_tree(self):
        repo = self._make_canonical_repo()
        self._run_git("-C", str(repo), "update-ref", "refs/remotes/origin/develop", "HEAD")
        expected_tree = subprocess.check_output(
            ["git", "-C", str(repo), "rev-parse", "refs/remotes/origin/develop^{tree}"],
            env=git_fixture_env(),
        ).decode("ascii").strip()
        (repo / "src/branch.rs").write_text("//! Only on the work branch.\n", encoding="utf-8")
        self._run_git("-C", str(repo), "add", "src/branch.rs")
        self._run_git("-C", str(repo), "-c", "user.name=gwt tests", "-c",
                      "user.email=gwt-tests@example.invalid", "commit", "--quiet", "-m", "work branch change")

        with mock.patch.object(runner, "_git_command", wraps=runner._git_command) as probe:
            snapshot = runner._canonical_git_tree(repo / "src")
        self.assertEqual(snapshot, {"reference": "origin/develop", "root_tree_oid": expected_tree,
                                    "tree_prefix": "src/"})
        self.assertLessEqual(probe.call_count, 3, [call.args[1] for call in probe.call_args_list])

        original_git = runner._git_command
        injected = []

        def ambiguous_boundary(root, args, input_bytes=None):
            completed = original_git(root, args, input_bytes)
            if list(args) == ["rev-parse", "--is-inside-work-tree", "--show-toplevel", "--show-prefix"]:
                injected.append(True)
                return subprocess.CompletedProcess(completed.args, completed.returncode,
                                                   completed.stdout + b"\n", completed.stderr)
            return completed

        with mock.patch.object(runner, "_git_command", side_effect=ambiguous_boundary):
            self.assertEqual(runner._canonical_git_tree(repo / "src"), snapshot)
        self.assertEqual(injected, [True], "ambiguous combined output must use the original probes")

    def test_canonical_ref_peel_failure_does_not_fallback_to_lower_ref(self):
        repo = self._make_canonical_repo()
        self._run_git(
            "-C",
            str(repo),
            "update-ref",
            "refs/remotes/origin/main",
            "HEAD",
        )
        (repo / "src" / "module_00.rs").write_text(
            "//! corrupt develop\nfn changed() {}\n", encoding="utf-8"
        )
        self._run_git("-C", str(repo), "add", "src/module_00.rs")
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
            "develop tip",
        )
        self._run_git(
            "-C",
            str(repo),
            "update-ref",
            "refs/remotes/origin/develop",
            "HEAD",
        )
        completed = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
            env=git_fixture_env(),
        )
        commit_oid = completed.stdout.strip()
        object_path = repo / ".git" / "objects" / commit_oid[:2] / commit_oid[2:]
        object_path.rename(object_path.with_name(f"{object_path.name}.missing"))

        with self.assertRaisesRegex(RuntimeError, "origin/develop"):
            runner._canonical_git_tree(repo)

        for reference in ("refs/remotes/origin/develop", "refs/remotes/origin/main"):
            self._run_git("-C", str(repo), "update-ref", "-d", reference)
        (repo / ".git/HEAD").write_text(f"{commit_oid}\n", encoding="ascii")
        with self.assertRaisesRegex(RuntimeError, "HEAD"):
            runner._canonical_git_tree(repo)

    def test_canonical_git_commands_disable_lazy_fetch_and_prompts(self):
        completed = subprocess.CompletedProcess(
            args=["git"], returncode=0, stdout=b"", stderr=b""
        )
        with mock.patch.dict(
            os.environ,
            {"GIT_DIR": "/wrong/repository", "GIT_WORK_TREE": "/wrong/worktree"},
        ):
            with mock.patch.object(
                runner.subprocess, "run", return_value=completed
            ) as run:
                runner._git_command(self.project, ["rev-parse", "HEAD"])

        environment = run.call_args.kwargs.get("env")
        self.assertIsNotNone(environment)
        self.assertEqual(environment.get("GIT_NO_LAZY_FETCH"), "1")
        self.assertEqual(environment.get("GIT_TERMINAL_PROMPT"), "0")
        self.assertNotIn("GIT_DIR", environment)
        self.assertNotIn("GIT_WORK_TREE", environment)

    def test_cas_reads_are_pure_and_atomic_writers_cleanup_failed_temps(self):
        result = self._build_protocol_v2(
            self.project,
            "0000000000000403",
            repo_hash="abc1234567890403",
        )
        self.assertTrue(result.get("ok"), result)
        cas_root = runner.resolve_file_index_v2_root(
            "abc1234567890403", db_root=self.db_root
        ) / "cas"
        cas_entry = next(cas_root.glob("*/*.json"))
        payload = json.loads(cas_entry.read_text(encoding="utf-8"))
        identity = {
            "cas_key": payload["cas_key"],
            "input_digest": payload["input_digest"],
        }
        before = cas_entry.read_bytes()
        with mock.patch.object(runner, "_write_json_atomic") as rewrite:
            vector = runner._read_cas_vector(
                cas_root,
                identity,
                payload["embedding_contract"],
                payload["dimension"],
            )
        self.assertIsNotNone(vector)
        rewrite.assert_not_called()
        self.assertEqual(cas_entry.read_bytes(), before)
        with mock.patch.object(runner, "_vector_checksum", wraps=runner._vector_checksum) as checksum:
            self.assertEqual(runner._read_cas_vector(
                cas_root, identity, payload["embedding_contract"], payload["dimension"],
                expected_manifest_checksum=payload["vector_checksum"],
            ), vector)
        checksum.assert_called_once()
        self.assertIsNone(runner._read_cas_vector(
            cas_root, identity, payload["embedding_contract"], payload["dimension"],
            expected_manifest_checksum="0" * 64,
        ))
        self.assertEqual(cas_entry.read_bytes(), before)

        failed_json = self.base / "atomic" / "payload.json"
        with mock.patch.object(runner.os, "replace", side_effect=OSError("publish")):
            with self.assertRaises(OSError):
                runner._write_json_atomic(failed_json, {"value": 1})
        self.assertEqual(list(failed_json.parent.glob(".*.tmp")), [])

        failed_cas_root = self.base / "failed-cas"
        failed_identity = {"cas_key": "a" * 64, "input_digest": "b" * 64}
        with mock.patch.object(runner.os, "replace", side_effect=OSError("publish")):
            with self.assertRaises(OSError):
                runner._write_cas_vector(
                    failed_cas_root,
                    failed_identity,
                    payload["embedding_contract"],
                    [0.0] * payload["dimension"],
                    payload["dimension"],
                )
        self.assertEqual(list(failed_cas_root.rglob("*.tmp")), [])

    def test_non_git_and_unborn_sources_use_empty_base_and_full_overlay(self):
        non_git = self._build_protocol_v2(self.project, WORKTREE_HASH)

        unborn = self.base / "unborn"
        unborn_src = unborn / "src"
        unborn_src.mkdir(parents=True)
        for index in range(8):
            (unborn_src / f"module_{index:02}.rs").write_text(
                f"//! module {index}\nfn feature_{index}() {{}}\n",
                encoding="utf-8",
            )
        self._run_git("init", "--quiet", str(unborn))
        unborn_result = self._build_protocol_v2(
            unborn,
            "0000000000000301",
            repo_hash="abc1234567890301",
        )

        for result in (non_git, unborn_result):
            with self.subTest(result=result):
                self.assertTrue(result.get("ok"), result)
                self.assertEqual(result.get("base_document_count"), 0, result)
                self.assertEqual(result.get("overlay_document_count"), 8, result)
                self.assertEqual(result.get("requested_embeddings"), 8, result)
                self.assertEqual(
                    result.get("requested_embeddings"),
                    result.get("embedding_cache_hits", 0)
                    + result.get("computed_embeddings", 0),
                    result,
                )

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

    def test_nested_project_root_uses_project_relative_base_record_ids(self):
        canonical_repo = self._make_canonical_repo()
        nested_project = canonical_repo / "src"

        result = self._build_protocol_v2(
            nested_project,
            "0000000000000201",
            repo_hash="abc1234567890201",
        )

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("base_document_count"), 8, result)
        self.assertEqual(
            result.get("overlay_document_count"),
            0,
            "canonical and visible records must use the same project-relative IDs",
        )

    def test_canonical_builder_filters_oversized_blobs_before_cat_file(self):
        canonical_repo = self._make_canonical_repo()
        oversized = canonical_repo / "src" / "oversized.txt"
        oversized.write_bytes(b"x" * (runner.MAX_FILE_SIZE + 1))
        self._run_git("-C", str(canonical_repo), "add", "src/oversized.txt")
        self._run_git(
            "-C",
            str(canonical_repo),
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "add oversized fixture",
        )

        original_cat_file = runner._git_cat_file_batch
        requested_paths = []

        def guarded_cat_file(root, entries):
            requested_paths.extend(entry[0] for entry in entries)
            self.assertNotIn("src/oversized.txt", requested_paths)
            return original_cat_file(root, entries)

        with mock.patch.object(
            runner, "_git_cat_file_batch", side_effect=guarded_cat_file
        ):
            result = self._build_protocol_v2(
                canonical_repo,
                "0000000000000202",
                repo_hash="abc1234567890202",
            )

        self.assertTrue(result.get("ok"), result)
        self.assertNotIn("src/oversized.txt", requested_paths)

    def test_aggregate_base_and_overlay_materialize_both_file_scopes(self):
        canonical_repo = self._make_canonical_repo()
        docs = canonical_repo / "docs"
        docs.mkdir()
        (docs / "guide.md").write_text(
            "# Canonical guide\nShared base documentation.\n", encoding="utf-8"
        )
        self._run_git("-C", str(canonical_repo), "add", "docs/guide.md")
        self._run_git(
            "-C",
            str(canonical_repo),
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "add canonical docs",
        )
        project = self.base / "aggregate-scopes"
        self._run_git("clone", "--quiet", str(canonical_repo), str(project))
        (project / "src" / "overlay.rs").write_text(
            "//! overlay code\nfn overlay() {}\n", encoding="utf-8"
        )
        (project / "docs" / "overlay.md").write_text(
            "# Overlay guide\nWorktree documentation.\n", encoding="utf-8"
        )

        result = self._build_protocol_v2(
            project,
            "0000000000000302",
            repo_hash="abc1234567890302",
        )
        self.assertTrue(result.get("ok"), result)
        base_dir = runner.resolve_file_index_v2_base_dir(
            "abc1234567890302",
            result["base_generation_id"],
            db_root=self.db_root,
        )
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            "abc1234567890302",
            "0000000000000302",
            result["overlay_generation_id"],
            db_root=self.db_root,
        )
        base_descriptor = json.loads(
            (base_dir / "descriptor.json").read_text(encoding="utf-8")
        )
        overlay_descriptor = json.loads(
            (overlay_dir / "descriptor.json").read_text(encoding="utf-8")
        )

        self.assertEqual(set(base_descriptor), {
            "schema_version",
            "kind",
            "base_generation_id",
            "repo_hash",
            "root_tree_oid",
            "canonical_ref",
            "compatibility",
            "files_generation",
            "files_docs_generation",
            "manifest_digest",
            "document_counts",
            "build_state",
            "created_at",
            "verified_at",
        })
        self.assertEqual(set(overlay_descriptor), {
            "schema_version",
            "kind",
            "overlay_generation_id",
            "repo_hash",
            "worktree_hash",
            "base_generation_id",
            "source_snapshot_id",
            "compatibility",
            "files_generation",
            "files_docs_generation",
            "files_shadow",
            "files_docs_shadow",
            "tombstones",
            "manifest_digest",
            "build_state",
            "created_at",
            "verified_at",
        })
        self.assertEqual(base_descriptor["document_counts"], {
            "files": 8,
            "files-docs": 1,
            "total": 9,
        })
        self.assertEqual(
            base_descriptor["files_generation"],
            {"store": "records", "collection": runner.V2_FILES_CODE_COLLECTION},
        )
        self.assertEqual(
            base_descriptor["files_docs_generation"],
            {"store": "records", "collection": runner.V2_FILES_DOCS_COLLECTION},
        )
        self.assertEqual(
            overlay_descriptor["files_generation"],
            {"store": "records", "collection": runner.V2_FILES_CODE_COLLECTION},
        )
        self.assertEqual(
            overlay_descriptor["files_docs_generation"],
            {"store": "records", "collection": runner.V2_FILES_DOCS_COLLECTION},
        )
        self.assertEqual(overlay_descriptor["files_shadow"], ["src/overlay.rs"])
        self.assertEqual(
            overlay_descriptor["files_docs_shadow"], ["docs/overlay.md"]
        )
        self.assertEqual(overlay_descriptor["tombstones"], [])

        for artifact_dir, expected_counts in (
            (base_dir, {runner.V2_FILES_CODE_COLLECTION: 8, runner.V2_FILES_DOCS_COLLECTION: 1}),
            (overlay_dir, {runner.V2_FILES_CODE_COLLECTION: 1, runner.V2_FILES_DOCS_COLLECTION: 1}),
        ):
            for collection_name, expected_count in expected_counts.items():
                self.assertEqual(
                    self._artifact_collection_count(artifact_dir, collection_name),
                    expected_count,
                )

    def test_concurrent_subprocess_builds_single_flight_cas_and_atomic_artifacts(self):
        canonical_repo = self._make_canonical_repo()
        worktrees = [
            (
                self._clone_canonical_worktree(
                    canonical_repo, "concurrent-worktree-a", overlay_index=1
                ),
                "0000000000000303",
            ),
            (
                self._clone_canonical_worktree(
                    canonical_repo, "concurrent-worktree-b", overlay_index=2
                ),
                "0000000000000304",
            ),
        ]
        barrier_root = self.base / "cold-miss-barrier"
        barrier_root.mkdir()
        driver = textwrap.dedent(
            """
            import json
            import os
            import sys
            import time
            from pathlib import Path

            sys.path.insert(0, os.environ["GWT_TEST_RUNNER_DIR"])
            import chroma_index_runner as runner

            original_resolve = runner._resolve_record_vector_batch
            first_resolve = True

            def synchronized_resolve(*args, **kwargs):
                global first_resolve
                if first_resolve:
                    first_resolve = False
                    barrier = Path(os.environ["GWT_TEST_BARRIER_ROOT"])
                    (barrier / f"{os.getpid()}.ready").write_text("ready")
                    deadline = time.monotonic() + 10
                    while len(list(barrier.glob("*.ready"))) < 2:
                        if time.monotonic() >= deadline:
                            raise TimeoutError("file-index-v2 cold-miss barrier timed out")
                        time.sleep(0.01)
                return original_resolve(*args, **kwargs)

            original_encode = runner._FakeEmbeddingModel.encode

            def slow_encode(self, texts, **kwargs):
                time.sleep(0.3)
                return original_encode(self, texts, **kwargs)

            runner._resolve_record_vector_batch = synchronized_resolve
            runner._FakeEmbeddingModel.encode = slow_encode
            descriptor = runner.default_file_index_compatibility()
            descriptor["document_contract"]["payload_builder_version"] = int(
                os.environ["GWT_TEST_DOCUMENT_CONTRACT_VERSION"]
            )
            result = runner.action_index_files_v2(
                project_root=os.environ["GWT_TEST_PROJECT_ROOT"],
                repo_hash=os.environ["GWT_TEST_REPO_HASH"],
                worktree_hash=os.environ["GWT_TEST_WORKTREE_HASH"],
                mode="full",
                db_root=Path(os.environ["GWT_TEST_DB_ROOT"]),
                scope="files",
                file_index_protocol="v2",
                compatibility_descriptor=descriptor,
            )
            print(json.dumps(result, sort_keys=True))
            """
        )
        processes = []
        for contract_version, (project, worktree_hash) in enumerate(worktrees, start=1):
            environment = os.environ.copy()
            environment.update(
                {
                    "GWT_TEST_RUNNER_DIR": str(Path(runner.__file__).resolve().parent),
                    "GWT_TEST_BARRIER_ROOT": str(barrier_root),
                    "GWT_TEST_PROJECT_ROOT": str(project),
                    "GWT_TEST_REPO_HASH": "abc1234567890303",
                    "GWT_TEST_WORKTREE_HASH": worktree_hash,
                    "GWT_TEST_DB_ROOT": str(self.db_root),
                    "GWT_TEST_DOCUMENT_CONTRACT_VERSION": str(contract_version),
                }
            )
            processes.append(
                subprocess.Popen(
                    [sys.executable, "-c", driver],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    env=environment,
                )
            )
        results = []
        communicated = set()
        try:
            for process in processes:
                stdout, stderr = process.communicate(timeout=30)
                communicated.add(process)
                self.assertEqual(
                    process.returncode,
                    0,
                    f"stdout:\n{stdout}\nstderr:\n{stderr}",
                )
                results.append(json.loads(stdout.splitlines()[-1]))
        except subprocess.TimeoutExpired as error:
            self.fail(f"concurrent file-index-v2 subprocess timed out: {error}")
        finally:
            for process in processes:
                if process.poll() is None:
                    process.kill()
                if process not in communicated:
                    with contextlib.suppress(subprocess.TimeoutExpired):
                        process.communicate(timeout=5)
                for stream in (process.stdout, process.stderr):
                    if stream is not None and not stream.closed:
                        stream.close()

        for result in results:
            self.assertTrue(result.get("ok"), result)
        self.assertEqual(len(list(barrier_root.glob("*.ready"))), 2)
        self.assertEqual(sum(result["requested_embeddings"] for result in results), 18)
        self.assertEqual(sum(result["computed_embeddings"] for result in results), 10)
        self.assertEqual(sum(result["embedding_cache_hits"] for result in results), 8)
        self.assertTrue(
            any(result["embedding_cache_hits"] > 0 for result in results),
            results,
        )
        for result in results:
            self.assertEqual(
                result["requested_embeddings"],
                result["computed_embeddings"] + result["embedding_cache_hits"],
                result,
            )
        self.assertEqual(
            len({result["base_generation_id"] for result in results}),
            2,
        )
        self.assertEqual(
            len({result["overlay_generation_id"] for result in results}),
            2,
        )

        v2_root = runner.resolve_file_index_v2_root(
            "abc1234567890303", db_root=self.db_root
        )
        self.assertFalse(list(v2_root.rglob("*.staging-*")))
        self.assertFalse(list(v2_root.rglob("*.quarantine-*")))
        artifact_counts = [
            (
                runner.resolve_file_index_v2_base_dir(
                    "abc1234567890303",
                    result["base_generation_id"],
                    db_root=self.db_root,
                ),
                8,
            )
            for result in results
        ]
        artifact_counts.extend(
            (
                runner.resolve_file_index_v2_overlay_dir(
                    "abc1234567890303",
                    worktree_hash,
                    result["overlay_generation_id"],
                    db_root=self.db_root,
                ),
                1,
            )
            for (_, worktree_hash), result in zip(worktrees, results)
        )
        for artifact_dir, expected_count in artifact_counts:
            self.assertTrue((artifact_dir / "descriptor.json").is_file())
            self.assertTrue((artifact_dir / "manifest.json").is_file())
            self.assertEqual(self._artifact_collection_count(artifact_dir), expected_count)
            descriptor = json.loads(
                (artifact_dir / "descriptor.json").read_text(encoding="utf-8")
            )
            manifest = json.loads(
                (artifact_dir / "manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(descriptor["build_state"], "verified")
            self.assertEqual(
                descriptor["manifest_digest"],
                runner._sha256_json(manifest["entries"]),
            )
            self.assertEqual(len(manifest["entries"]), expected_count)

    def test_same_target_concurrent_publish_converges_on_one_verified_artifact(self):
        canonical_repo = self._make_canonical_repo()
        project = self.base / "same-target-concurrent-worktree"
        self._run_git("clone", "--quiet", str(canonical_repo), str(project))
        barrier_root = self.base / "same-target-publish-barrier"
        barrier_root.mkdir()
        driver = textwrap.dedent(
            """
            import json
            import os
            import sys
            import time
            from pathlib import Path

            sys.path.insert(0, os.environ["GWT_TEST_RUNNER_DIR"])
            import chroma_index_runner as runner

            original_materialize = runner._materialize_file_artifact_pair
            first_materialize = True

            def synchronized_materialize(*args, **kwargs):
                global first_materialize
                if first_materialize:
                    first_materialize = False
                    barrier = Path(os.environ["GWT_TEST_BARRIER_ROOT"])
                    (barrier / f"{os.getpid()}.ready").write_text("ready")
                    deadline = time.monotonic() + 10
                    while len(list(barrier.glob("*.ready"))) < 2:
                        if time.monotonic() >= deadline:
                            raise TimeoutError("file-index-v2 publish barrier timed out")
                        time.sleep(0.01)
                return original_materialize(*args, **kwargs)

            original_encode = runner._FakeEmbeddingModel.encode

            def slow_encode(self, texts, **kwargs):
                time.sleep(0.3)
                return original_encode(self, texts, **kwargs)

            runner._materialize_file_artifact_pair = synchronized_materialize
            runner._FakeEmbeddingModel.encode = slow_encode
            result = runner.action_index_files_v2(
                project_root=os.environ["GWT_TEST_PROJECT_ROOT"],
                repo_hash=os.environ["GWT_TEST_REPO_HASH"],
                worktree_hash=os.environ["GWT_TEST_WORKTREE_HASH"],
                mode="full",
                db_root=Path(os.environ["GWT_TEST_DB_ROOT"]),
                scope="files",
                file_index_protocol="v2",
            )
            print(json.dumps(result, sort_keys=True))
            """
        )
        environment = os.environ.copy()
        environment.update(
            {
                "GWT_TEST_RUNNER_DIR": str(Path(runner.__file__).resolve().parent),
                "GWT_TEST_BARRIER_ROOT": str(barrier_root),
                "GWT_TEST_PROJECT_ROOT": str(project),
                "GWT_TEST_REPO_HASH": "abc1234567890305",
                "GWT_TEST_WORKTREE_HASH": "0000000000000305",
                "GWT_TEST_DB_ROOT": str(self.db_root),
            }
        )
        processes = [
            subprocess.Popen(
                [sys.executable, "-c", driver],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=environment,
            )
            for _ in range(2)
        ]
        results = []
        communicated = set()
        try:
            for process in processes:
                stdout, stderr = process.communicate(timeout=30)
                communicated.add(process)
                self.assertEqual(
                    process.returncode,
                    0,
                    f"stdout:\n{stdout}\nstderr:\n{stderr}",
                )
                results.append(json.loads(stdout.splitlines()[-1]))
        except subprocess.TimeoutExpired as error:
            self.fail(f"same-target file-index-v2 subprocess timed out: {error}")
        finally:
            for process in processes:
                if process.poll() is None:
                    process.kill()
                if process not in communicated:
                    with contextlib.suppress(subprocess.TimeoutExpired):
                        process.communicate(timeout=5)
                for stream in (process.stdout, process.stderr):
                    if stream is not None and not stream.closed:
                        stream.close()

        self.assertEqual(len(results), 2)
        self.assertEqual(len({item["base_generation_id"] for item in results}), 1)
        self.assertEqual(len({item["overlay_generation_id"] for item in results}), 1)
        self.assertEqual(sum(item["requested_embeddings"] for item in results), 16)
        self.assertEqual(sum(item["computed_embeddings"] for item in results), 8)
        self.assertEqual(sum(item["embedding_cache_hits"] for item in results), 8)
        self.assertEqual(len(list(barrier_root.glob("*.ready"))), 2)

        v2_root = runner.resolve_file_index_v2_root(
            "abc1234567890305", db_root=self.db_root
        )
        self.assertFalse(list(v2_root.rglob("*.staging-*")))
        self.assertFalse(list(v2_root.rglob("*.quarantine-*")))
        base_dir = runner.resolve_file_index_v2_base_dir(
            "abc1234567890305",
            results[0]["base_generation_id"],
            db_root=self.db_root,
        )
        overlay_dir = runner.resolve_file_index_v2_overlay_dir(
            "abc1234567890305",
            "0000000000000305",
            results[0]["overlay_generation_id"],
            db_root=self.db_root,
        )
        for artifact_dir, expected_count in ((base_dir, 8), (overlay_dir, 0)):
            descriptor = json.loads(
                (artifact_dir / "descriptor.json").read_text(encoding="utf-8")
            )
            manifest = json.loads(
                (artifact_dir / "manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(descriptor["build_state"], "verified")
            self.assertEqual(
                descriptor["manifest_digest"],
                runner._sha256_json(manifest["entries"]),
            )
            self.assertEqual(len(manifest["entries"]), expected_count)

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
                self.assertEqual(self._artifact_collection_count(base_dir), 8)
                self.assertEqual(self._artifact_collection_count(overlay_dir), 1)
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

    def test_unchanged_copy_resolves_generation_under_shared_lock(self):
        self.assertTrue(self._build().get("ok"))
        db_path = self._db_path()
        hashes = {
            entry["path"]: entry["content_hash"]
            for entry in runner.read_manifest(db_path, scope="files")
        }
        real_lock, real_resolve = runner.acquire_lock, runner.resolve_active_store
        real_manifest = runner.read_manifest
        held = False

        @contextlib.contextmanager
        def shared_lock(path, exclusive=True):
            nonlocal held
            self.assertEqual((path, exclusive), (db_path, False))
            with real_lock(path, exclusive=exclusive):
                held = True
                yield
                held = False

        def resolve_locked(path):
            self.assertTrue(held, "generation selection must follow reader admission")
            return real_resolve(path)

        def manifest_locked(path, scope):
            self.assertTrue(held, "manifest and generation must share reader admission")
            return real_manifest(path, scope)

        with mock.patch.object(
            runner, "acquire_lock", side_effect=shared_lock
        ), mock.patch.object(
            runner, "resolve_active_store", side_effect=resolve_locked
        ), mock.patch.object(
            runner, "read_manifest", side_effect=manifest_locked
        ):
            copied = runner._copy_unchanged_records(
                db_path, "files", mock.Mock(), "files_code", hashes
            )
        self.assertEqual(copied, len(hashes))


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
