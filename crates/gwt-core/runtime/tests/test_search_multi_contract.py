"""Phase 70 T-IDX-389 (Issue #3264): versioned search-multi contract.

FR-384 / AS-2: one search-multi request encodes the query once and reuses
the embedding across every scope. FR-387/FR-388: per-scope classification
(fresh / stale / missing / corrupt) is reported instead of failing the whole
batch or silently returning empty results for broken scopes.
"""

from __future__ import annotations

import datetime
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


class SearchMultiContractTests(unittest.TestCase):
    def setUp(self):
        runner._MODEL_CACHE = None
        self._coord_tmp = tempfile.TemporaryDirectory()
        self._env = mock.patch.dict(
            os.environ,
            {
                "GWT_INDEX_COORDINATOR_ROOT": self._coord_tmp.name,
                "GWT_INDEX_FAKE_EMBEDDING": "1",
            },
            clear=False,
        )
        self._env.start()

    def tearDown(self):
        self._env.stop()
        self._coord_tmp.cleanup()
        runner._MODEL_CACHE = None

    def _seed_file_scopes(self, base: Path, db_root: Path) -> Path:
        project = base / "project"
        (project / "src").mkdir(parents=True)
        (project / "src" / "alpha.rs").write_text(
            "//! alpha searcher module\nfn alpha_search() {}\n", encoding="utf-8"
        )
        (project / "docs").mkdir()
        (project / "docs" / "guide.md").write_text(
            "# Guide\nalpha search documentation.\n", encoding="utf-8"
        )
        for scope in ("files", "files-docs"):
            result = runner.action_index_files_v2(
                project_root=str(project),
                repo_hash=REPO_HASH,
                worktree_hash=WORKTREE_HASH,
                mode="full",
                db_root=db_root,
                scope=scope,
            )
            self.assertTrue(result.get("ok"), result)
        return project

    def test_search_multi_encodes_query_once_across_scopes(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = self._seed_file_scopes(base, db_root)

            real_model = runner._FakeEmbeddingModel()
            counting = mock.MagicMock()
            counting.encode.side_effect = real_model.encode
            with mock.patch.object(
                runner, "_get_embedding_model", return_value=counting
            ):
                payload = runner.action_search_multi_v2(
                    repo_hash=REPO_HASH,
                    worktree_hash=WORKTREE_HASH,
                    project_root=str(project),
                    query="alpha search",
                    n_results=5,
                    scopes=["files", "files-docs"],
                    db_root=db_root,
                )
            self.assertTrue(payload.get("ok"), payload)
            self.assertEqual(
                counting.encode.call_count,
                1,
                "search-multi must encode the query once and reuse the "
                f"embedding across scopes (AS-2), calls: {counting.encode.call_args_list}",
            )

    def test_search_multi_classifies_missing_and_corrupt_scopes(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = self._seed_file_scopes(base, db_root)
            # Corrupt the files-docs store; leave files healthy; query a
            # third scope that was never built (issues) as missing.
            docs_db = runner.resolve_db_path(
                REPO_HASH, WORKTREE_HASH, "files-docs", db_root=db_root
            )
            docs_store = runner.resolve_active_store(docs_db)
            (docs_store / "chroma.sqlite3").write_bytes(b"corrupt-not-a-database")

            payload = runner.action_search_multi_v2(
                repo_hash=REPO_HASH,
                worktree_hash=WORKTREE_HASH,
                project_root=str(project),
                query="alpha search",
                n_results=5,
                scopes=["issues", "files", "files-docs"],
                db_root=db_root,
            )

            self.assertTrue(
                payload.get("ok"),
                f"classification must not fail the whole batch: {payload}",
            )
            scopes = payload.get("scopes") or {}
            self.assertEqual(
                scopes.get("issues", {}).get("state"),
                "missing",
                f"never-built scope must classify as missing: {payload}",
            )
            self.assertEqual(
                scopes.get("files", {}).get("state"),
                "fresh",
                f"healthy scope must classify as fresh: {payload}",
            )
            self.assertEqual(
                scopes.get("files-docs", {}).get("state"),
                "corrupt",
                f"unreadable store must classify as corrupt, not silent-empty: {payload}",
            )

    def test_search_multi_marks_ttl_expired_issues_scope_stale(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            cache_root = base / ".gwt" / "cache" / "issues" / REPO_HASH
            issue_dir = cache_root / "1"
            issue_dir.mkdir(parents=True)
            (issue_dir / "meta.json").write_text(
                json.dumps(
                    {
                        "number": 1,
                        "title": "First issue about alpha search",
                        "labels": ["bug"],
                        "state": "open",
                        "updated_at": "2026-07-01T00:00:00Z",
                        "comment_ids": [],
                    }
                ),
                encoding="utf-8",
            )
            (issue_dir / "body.md").write_text(
                "alpha search regression details", encoding="utf-8"
            )

            with mock.patch.dict(os.environ, {"HOME": str(base)}, clear=False):
                result = runner.action_index_issues_v2(
                    repo_hash=REPO_HASH,
                    project_root=str(base),
                    db_root=db_root,
                    respect_ttl=False,
                )
                self.assertTrue(result.get("ok"), result)

                # Age the index past its TTL: healthy store, stale freshness.
                meta_path = db_root / REPO_HASH / "issues" / "meta.json"
                meta = json.loads(meta_path.read_text(encoding="utf-8"))
                stale_at = datetime.datetime.now(
                    datetime.timezone.utc
                ) - datetime.timedelta(hours=1)
                meta["last_full_refresh"] = stale_at.isoformat()
                meta_path.write_text(json.dumps(meta), encoding="utf-8")

                payload = runner.action_search_multi_v2(
                    repo_hash=REPO_HASH,
                    worktree_hash=None,
                    project_root=str(base),
                    query="alpha search",
                    n_results=5,
                    scopes=["issues"],
                    db_root=db_root,
                )

            self.assertTrue(payload.get("ok"), payload)
            scopes = payload.get("scopes") or {}
            self.assertEqual(
                scopes.get("issues", {}).get("state"),
                "stale",
                f"TTL-expired healthy scope must classify as stale: {payload}",
            )
            self.assertIn(
                "issues",
                payload.get("stale_scopes") or [],
                f"stale scopes must be listed additively (FR-387): {payload}",
            )
            self.assertTrue(
                payload.get("issueResults"),
                f"stale scopes still serve verified results (FR-387): {payload}",
            )


if __name__ == "__main__":
    unittest.main()
