"""Phase 6: tests for richer file/docs payload construction."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
