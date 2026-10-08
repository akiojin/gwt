"""Phase 8: tests for the E5 prefix injection in the embedding function.

multilingual-e5-base requires "passage: " for documents and "query: " for queries.
The runner must transparently inject these prefixes via a custom EmbeddingFunction.
"""

from __future__ import annotations

import hashlib
import math
import unittest
from unittest import mock

import numpy

import chroma_index_runner as runner


class EmbeddingCasKeyTests(unittest.TestCase):
    def _embedding_contract(self) -> dict:
        return {
            "model_id": "intfloat/multilingual-e5-base",
            "revision": "model-revision-a",
            "dimension": 768,
            "normalization": "none",
            "metric": "cosine",
            "query_prefix": "query: ",
            "passage_prefix": "passage: ",
        }

    def _model_input(
        self,
        rel_path: str = "src/lib.rs",
        text: str = "pub fn shared_document() { 1 }",
    ) -> str:
        document = runner.build_embedding_document(
            rel_path=rel_path,
            description="shared index document",
            text=text,
            bucket="code",
            file_type="rs",
        )
        return f"passage: {document}"

    def _cas_identity(self, contract: dict, model_input: str) -> dict:
        key_builder = getattr(runner, "file_embedding_cas_key", None)
        self.assertIsNotNone(
            key_builder,
            "Phase 71 requires file_embedding_cas_key for exact model inputs",
        )
        return key_builder(contract, model_input)

    def test_cas_identity_uses_full_sha256_of_final_model_input(self):
        model_input = self._model_input()
        identity = self._cas_identity(self._embedding_contract(), model_input)
        expected_input_digest = hashlib.sha256(model_input.encode("utf-8")).hexdigest()

        self.assertEqual(identity.get("input_digest"), expected_input_digest)
        self.assertIsInstance(identity.get("cas_key"), str)
        self.assertTrue(identity["cas_key"], "CAS identity must provide a stable key")

    def test_cas_key_changes_for_every_embedding_contract_field(self):
        baseline = self._embedding_contract()
        model_input = self._model_input()
        baseline_identity = self._cas_identity(baseline, model_input)
        changes = {
            "model_id": "different-model",
            "revision": "model-revision-b",
            "dimension": 384,
            "normalization": "l2",
            "metric": "dot",
            "query_prefix": "search: ",
            "passage_prefix": "document: ",
        }

        for field, replacement in changes.items():
            with self.subTest(field=field):
                changed = dict(baseline)
                changed[field] = replacement
                changed_identity = self._cas_identity(changed, model_input)
                self.assertEqual(
                    baseline_identity.get("input_digest"),
                    changed_identity.get("input_digest"),
                    "the exact final input did not change in this fixture",
                )
                self.assertNotEqual(
                    baseline_identity.get("cas_key"),
                    changed_identity.get("cas_key"),
                    f"{field} must invalidate embedding CAS identity",
                )

    def test_cas_key_changes_with_path_aware_structured_document(self):
        contract = self._embedding_contract()
        original = self._cas_identity(contract, self._model_input("src/lib.rs"))
        renamed = self._cas_identity(contract, self._model_input("src/renamed.rs"))

        self.assertNotEqual(
            original.get("input_digest"),
            renamed.get("input_digest"),
            "a rename changes the exact passage input and must miss the CAS",
        )
        self.assertNotEqual(original.get("cas_key"), renamed.get("cas_key"))

    def test_cas_identity_changes_when_body_differs_by_one_byte(self):
        contract = self._embedding_contract()
        first_input = self._model_input(text="pub fn shared_document() { 1 }")
        second_input = self._model_input(text="pub fn shared_document() { 2 }")
        first_bytes = first_input.encode("utf-8")
        second_bytes = second_input.encode("utf-8")
        self.assertEqual(len(first_bytes), len(second_bytes))
        self.assertEqual(
            sum(left != right for left, right in zip(first_bytes, second_bytes)),
            1,
            "the fixture must change exactly one final-input byte",
        )

        first = self._cas_identity(contract, first_input)
        second = self._cas_identity(contract, second_input)
        self.assertNotEqual(first.get("input_digest"), second.get("input_digest"))
        self.assertNotEqual(first.get("cas_key"), second.get("cas_key"))


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

    def test_embed_query_normalizes_numpy_scalars_to_native_finite_floats(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = numpy.asarray(
            [[0.1, 0.2, 0.3]], dtype=numpy.float32
        )

        result = runner.E5EmbeddingFunction(model=fake_model).embed_query("query")

        self.assertTrue(result)
        self.assertTrue(
            all(type(value) is float for value in result[0]),
            f"Chroma query embeddings require native Python floats: {result!r}",
        )
        self.assertTrue(all(math.isfinite(value) for value in result[0]))

    def test_embed_query_rejects_non_finite_values_before_chroma(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = numpy.asarray(
            [[0.1, numpy.nan]], dtype=numpy.float32
        )

        with self.assertRaisesRegex(ValueError, "finite"):
            runner.E5EmbeddingFunction(model=fake_model).embed_query("query")

    def test_embed_documents_keeps_existing_model_scalar_conversion_path(self):
        fake_model = mock.MagicMock()
        fake_model.encode.return_value = numpy.asarray(
            [[0.1, 0.2]], dtype=numpy.float32
        )

        result = runner.E5EmbeddingFunction(model=fake_model).embed_documents("document")

        self.assertIsInstance(
            result[0][0],
            numpy.float32,
            "FR-399 normalizes only the precomputed query boundary; document "
            "embedding must not gain an extra per-scalar Python conversion",
        )


if __name__ == "__main__":
    unittest.main()
