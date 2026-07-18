"""SPEC-1939 Phase 17: Index search match modes."""

from __future__ import annotations

import unittest
from unittest import mock

import chroma_index_runner as runner


class SearchMatchModeTests(unittest.TestCase):
    def test_parse_required_terms_preserves_quoted_phrases_and_no_space_japanese(self):
        self.assertEqual(
            runner._parse_required_terms('Workspace "Project State" 移行'),
            ["Workspace", "Project State", "移行"],
        )
        self.assertEqual(
            runner._parse_required_terms("Workspace置き換え"),
            ["Workspace置き換え"],
        )

    def test_all_terms_split_keeps_strict_results_separate_from_suggestions(self):
        items = [
            {
                "id": "strict",
                "document": "Workspace terminology replacement keeps 置き換え history.",
                "metadata": {"title": "Workspace 置き換え"},
                "distance": 0.1,
            },
            {
                "id": "suggestion",
                "document": "Workspace terminology only.",
                "metadata": {"title": "Workspace"},
                "distance": 0.2,
            },
        ]

        strict, suggestions = runner._apply_match_mode(
            items,
            query="Workspace 置き換え",
            match_mode="all_terms",
        )

        self.assertEqual([item["id"] for item in strict], ["strict"])
        self.assertEqual([item["id"] for item in suggestions], ["suggestion"])
        self.assertEqual(strict[0]["matched_terms"], ["Workspace", "置き換え"])
        self.assertEqual(strict[0]["missing_terms"], [])
        self.assertEqual(suggestions[0]["matched_terms"], ["Workspace"])
        self.assertEqual(suggestions[0]["missing_terms"], ["置き換え"])

    def test_search_multi_returns_suggestions_by_scope(self):
        with mock.patch.object(
            runner,
            "_classify_scope_for_search",
            return_value=("fresh", {"reason": "ready"}),
        ), mock.patch.object(
            runner,
            "_search_scope_collection",
            return_value={
                "issueResults": [],
                "suggestions": [{"number": 1, "title": "Workspace only"}],
            },
        ) as search:
            result = runner.action_search_multi_v2(
                repo_hash="repo",
                worktree_hash=None,
                project_root="/repo",
                query="Workspace 置き換え",
                n_results=10,
                scopes=["issues"],
                match_mode="all_terms",
            )

        self.assertTrue(result["ok"])
        self.assertEqual(result["suggestions"]["issues"][0]["title"], "Workspace only")
        self.assertEqual(search.call_args.args[5], "all_terms")


if __name__ == "__main__":
    unittest.main()
