"""Phase 8: tests for the E5 prefix injection in the embedding function.

multilingual-e5-base requires "passage: " for documents and "query: " for queries.
The runner must transparently inject these prefixes via a custom EmbeddingFunction.
"""

from __future__ import annotations

import unittest
from unittest import mock

import chroma_index_runner as runner


class E5PrefixTests(unittest.TestCase):
    def test_embedding_function_class_exists(self):
        self.assertTrue(
            hasattr(runner, "E5EmbeddingFunction"),
            "Phase 8 must add E5EmbeddingFunction class to the runner",
        )

    def test_embed_documents_prepends_passage_prefix(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = [[0.1] * 768, [0.2] * 768]

        ef = runner.E5EmbeddingFunction(model=fake_model)
        ef.embed_documents(["hello world", "second doc"])

        call_args = fake_model.encode.call_args
        passed = call_args[0][0]
        self.assertEqual(passed[0], "passage: hello world")
        self.assertEqual(passed[1], "passage: second doc")

    def test_embed_query_prepends_query_prefix(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = [[0.1] * 768]

        ef = runner.E5EmbeddingFunction(model=fake_model)
        ef.embed_query("how do watchers work")

        call_args = fake_model.encode.call_args
        passed = call_args[0][0]
        self.assertEqual(passed[0], "query: how do watchers work")

    def test_passage_prefix_not_double_applied(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = [[0.1] * 768]

        ef = runner.E5EmbeddingFunction(model=fake_model)
        ef.embed_documents(["passage: already prefixed"])

        passed = fake_model.encode.call_args[0][0]
        self.assertEqual(
            passed[0],
            "passage: already prefixed",
            "Existing passage: prefix must not be doubled",
        )

    def test_query_prefix_not_double_applied(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = [[0.1] * 768]

        ef = runner.E5EmbeddingFunction(model=fake_model)
        ef.embed_query("query: already prefixed")

        passed = fake_model.encode.call_args[0][0]
        self.assertEqual(passed[0], "query: already prefixed")

    def test_chroma_compatibility_call_protocol(self):
        """Chroma's EmbeddingFunction calls __call__(input) and expects list of vectors.

        The class must support both Chroma's EmbeddingFunction __call__ protocol
        AND our internal embed_documents / embed_query split.
        """
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = [[0.5] * 768, [0.6] * 768]

        ef = runner.E5EmbeddingFunction(model=fake_model)
        # Chroma compatibility — __call__ should default to passage mode
        result = ef(["doc one", "doc two"])
        self.assertEqual(len(result), 2)


if __name__ == "__main__":
    unittest.main()
