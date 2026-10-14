"""Phase 6: tests for richer file/docs payload construction."""

from __future__ import annotations

import hashlib
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class RecordingCollection:
    def __init__(self) -> None:
        self.ids = []
        self.documents = []
        self.metadatas = []

    def upsert(self, ids, documents, metadatas) -> None:
        self.ids.extend(ids)
        self.documents.extend(documents)
        self.metadatas.extend(metadatas)


class FilePayloadTests(unittest.TestCase):
    def test_embed_documents_for_paths_uses_structured_code_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            src = root / "src"
            src.mkdir(parents=True)
            file_path = src / "watcher.rs"
            file_path.write_text(
                "//! file system watcher with debounce\n"
                "fn debounce_events() {}\n"
            )

            collection = RecordingCollection()
            count = runner.embed_documents_for_paths([file_path], root, collection)

            self.assertEqual(count, 1)
            document = collection.documents[0]
            metadata = collection.metadatas[0]
            self.assertIn("path: src/watcher.rs", document)
            self.assertIn("bucket: code", document)
            self.assertIn("description: file system watcher with debounce", document)
            self.assertIn("content:", document)
            self.assertEqual(metadata["bucket"], "code")

    def test_embed_documents_for_paths_uses_structured_docs_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir(parents=True)
            file_path = root / "README.md"
            file_path.write_text(
                "# Project index health\n"
                "Docs repair details live here.\n"
            )

            collection = RecordingCollection()
            count = runner.embed_documents_for_paths([file_path], root, collection)

            self.assertEqual(count, 1)
            document = collection.documents[0]
            metadata = collection.metadatas[0]
            self.assertIn("path: README.md", document)
            self.assertIn("bucket: docs", document)
            self.assertIn("description: Project index health", document)
            self.assertIn("Docs repair details live here.", document)
            self.assertEqual(metadata["bucket"], "docs")


class ExactModelInputCasIntegrationTests(unittest.TestCase):
    def setUp(self):
        runner._MODEL_CACHE = None
        self._tmp = tempfile.TemporaryDirectory()
        self.base = Path(self._tmp.name)
        coordinator = self.base / "coordinator"
        coordinator.mkdir()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": str(coordinator),
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._tmp.cleanup()
        runner._MODEL_CACHE = None

    def test_cas_hashes_the_exact_passage_bytes_sent_to_model_encode(self):
        key_builder = getattr(runner, "file_embedding_cas_key", None)
        self.assertIsNotNone(
            key_builder,
            "Phase 71 requires file_embedding_cas_key for exact model inputs",
        )

        project = self.base / "project"
        src = project / "src"
        src.mkdir(parents=True)
        (src / "identity.rs").write_text(
            "//! exact model input identity\nfn identity() {}\n",
            encoding="utf-8",
        )

        cas_inputs = []
        cas_identities = []

        def record_cas_identity(contract, model_input):
            identity = key_builder(contract, model_input)
            cas_inputs.append(model_input.encode("utf-8"))
            cas_identities.append(identity)
            return identity

        real_model = runner._FakeEmbeddingModel()
        counting = mock.MagicMock()
        encoded_inputs = []

        def record_encode(texts, **kwargs):
            encoded_inputs.extend(texts)
            return real_model.encode(texts, **kwargs)

        counting.encode.side_effect = record_encode
        try:
            with mock.patch.object(
                runner,
                "file_embedding_cas_key",
                side_effect=record_cas_identity,
            ):
                with mock.patch.object(
                    runner,
                    "_get_embedding_model",
                    return_value=counting,
                ):
                    result = runner.action_index_files_v2(
                        project_root=str(project),
                        repo_hash="abc1234567890def",
                        worktree_hash="111122223333ffff",
                        mode="full",
                        db_root=self.base / "index",
                        scope="files",
                        file_index_protocol="v2",
                    )
        except TypeError as error:
            self.fail(
                "Phase 71 explicit v2 file index protocol is not implemented: "
                f"{error}"
            )

        self.assertTrue(result.get("ok"), result)
        self.assertEqual(result.get("indexed"), 1, result)
        self.assertEqual(result.get("computed_embeddings"), 1, result)
        self.assertTrue(cas_inputs, "the CAS key builder must observe the model input")
        self.assertTrue(encoded_inputs, "a cold CAS must call model encode")
        self.assertEqual(
            len(encoded_inputs),
            result.get("computed_embeddings"),
            "computed_embeddings must match the number of encoded passages",
        )

        encoded_bytes = [item.encode("utf-8") for item in encoded_inputs]
        self.assertEqual(
            len(cas_inputs),
            len(encoded_bytes),
            "every encoded passage must have exactly one CAS identity",
        )
        self.assertEqual(
            cas_inputs,
            encoded_bytes,
            "CAS identity must hash exactly the passage bytes passed to encode",
        )
        self.assertTrue(
            all(item.startswith(b"passage: ") for item in cas_inputs),
            f"CAS identity must include the final E5 prefix: {cas_inputs!r}",
        )
        for cas_input, identity in zip(cas_inputs, cas_identities):
            self.assertEqual(
                identity.get("input_digest"),
                hashlib.sha256(cas_input).hexdigest(),
            )


if __name__ == "__main__":
    unittest.main()
