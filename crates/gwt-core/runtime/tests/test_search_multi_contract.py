"""Phase 70 T-IDX-389 (Issue #3264): versioned search-multi contract.

FR-384 / AS-2: one search-multi request encodes the query once and reuses
the embedding across every scope. FR-387/FR-388: per-scope classification
(fresh / stale / missing / corrupt) is reported instead of failing the whole
batch or silently returning empty results for broken scopes.
"""

from __future__ import annotations

import argparse
import datetime
import json
import math
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import numpy

import chroma_index_runner as runner

REPO_HASH = "abc1234567890def"
WORKTREE_HASH = "111122223333ffff"


class _KnownDistanceEmbeddingModel:
    """Return 16-dimensional unit vectors with exact query cosine distances."""

    DIMENSION = 16

    def __init__(self, marker_distances: dict[str, float]) -> None:
        self._marker_distances = marker_distances
        self.query_encode_count = 0

    @classmethod
    def _vector_for_distance(cls, distance: float) -> list[float]:
        cosine = 1.0 - distance
        vector = [cosine, math.sqrt(max(0.0, 1.0 - cosine * cosine))]
        return vector + [0.0] * (cls.DIMENSION - len(vector))

    def encode(self, values, **_):
        vectors = []
        for value in values:
            text = str(value)
            if text.startswith("query: "):
                self.query_encode_count += 1
                vectors.append([1.0] + [0.0] * (self.DIMENSION - 1))
                continue
            marker = next(
                (candidate for candidate in self._marker_distances if candidate in text),
                None,
            )
            if marker is None:
                raise AssertionError(f"fixture document has no distance marker: {text}")
            vectors.append(self._vector_for_distance(self._marker_distances[marker]))
        return vectors


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

    def _run_fixture_git(self, project: Path, *arguments: str) -> None:
        git_env = os.environ.copy()
        git_env["GIT_CONFIG_NOSYSTEM"] = "1"
        git_env["GIT_CONFIG_GLOBAL"] = os.devnull
        git_env["GIT_ATTR_NOSYSTEM"] = "1"
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
            git_env.pop(key, None)
        for key in list(git_env):
            if (
                key == "GIT_CONFIG_COUNT"
                or key.startswith("GIT_CONFIG_KEY_")
                or key.startswith("GIT_CONFIG_VALUE_")
            ):
                git_env.pop(key)
        command = ["git", "-C", str(project), *arguments]
        try:
            completed = subprocess.run(
                command,
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
            f"{' '.join(command)} failed\n{completed.stderr}",
        )

    @staticmethod
    def _write_fixture_sources(project: Path, sources: dict[str, str | None]) -> None:
        for relative, content in sources.items():
            path = project / relative
            if content is None:
                path.unlink()
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")

    def _build_v2_view_fixture(
        self,
        base: Path,
        canonical_sources: dict[str, str],
        dirty_sources: dict[str, str | None],
        marker_distances: dict[str, float],
    ) -> tuple[Path, Path, _KnownDistanceEmbeddingModel]:
        project = base / "project"
        project.mkdir()
        self._write_fixture_sources(project, canonical_sources)
        self._run_fixture_git(project, "init", "--quiet")
        self._run_fixture_git(project, "symbolic-ref", "HEAD", "refs/heads/develop")
        self._run_fixture_git(project, "add", ".")
        self._run_fixture_git(
            project,
            "-c",
            "user.name=gwt tests",
            "-c",
            "user.email=gwt-tests@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "canonical search fixture",
        )
        self._write_fixture_sources(project, dirty_sources)

        db_root = base / "index"
        model = _KnownDistanceEmbeddingModel(marker_distances)
        compatibility = runner.default_file_index_compatibility()
        compatibility["dimension"] = model.DIMENSION
        with mock.patch.object(
            runner, "_get_embedding_model", return_value=model
        ), mock.patch.object(
            runner,
            "_expected_file_index_vector_dimension",
            return_value=model.DIMENSION,
        ):
            result = runner.action_index_files_v2(
                project_root=str(project),
                repo_hash=REPO_HASH,
                worktree_hash=WORKTREE_HASH,
                mode="full",
                db_root=db_root,
                scope="files",
                file_index_protocol="v2",
                compatibility_descriptor=compatibility,
            )
        self.assertTrue(result.get("ok"), result)
        self.assertTrue(result.get("published") or result.get("already_active"), result)
        model.query_encode_count = 0
        return project, db_root, model

    def _search_v2_view(
        self,
        project: Path,
        db_root: Path,
        model: _KnownDistanceEmbeddingModel,
        *,
        query: str = "alpha",
        n_results: int = 10,
        scopes: tuple[str, ...] = ("files",),
        match_mode: str = "semantic",
    ) -> dict:
        with mock.patch.object(runner, "_get_embedding_model", return_value=model):
            return runner.action_search_multi_v2(
                repo_hash=REPO_HASH,
                worktree_hash=WORKTREE_HASH,
                project_root=str(project),
                query=query,
                n_results=n_results,
                scopes=scopes,
                db_root=db_root,
                match_mode=match_mode,
                file_index_protocol="v2",
            )

    @staticmethod
    def _scope_file_results(payload: dict, scope: str) -> list[dict]:
        return list(
            ((payload.get("scope_results") or {}).get(scope) or {}).get("results")
            or []
        )

    def test_v2_view_search_excludes_tombstone_and_uses_overlay_authority(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, db_root, model = self._build_v2_view_fixture(
                Path(tmp),
                {
                    "src/deleted.rs": "//! MARK_DELETED alpha beta\n",
                    "src/changed.rs": "//! MARK_CHANGED_BASE alpha beta\n",
                    "src/rename.rs": "//! MARK_RENAME alpha beta\n",
                    "src/keep.rs": "//! MARK_KEEP alpha beta\n",
                },
                {
                    "src/deleted.rs": None,
                    "src/changed.rs": "//! MARK_CHANGED_OVERLAY alpha\n",
                    "src/rename.rs": None,
                    "src/renamed.rs": "//! MARK_RENAME alpha beta\n",
                    "src/near.rs": "//! MARK_NEAR_OVERLAY alpha beta\n",
                },
                {
                    "MARK_DELETED": 0.01,
                    "MARK_CHANGED_BASE": 0.02,
                    "MARK_NEAR_OVERLAY": 0.005,
                    "MARK_RENAME": 0.15,
                    "MARK_CHANGED_OVERLAY": 0.9,
                    "MARK_KEEP": 0.3,
                },
            )

            payload = self._search_v2_view(
                project, db_root, model, n_results=2
            )

            self.assertTrue(payload.get("ok"), payload)
            results = self._scope_file_results(payload, "files")
            paths = [item["path"] for item in results]
            self.assertNotIn("src/deleted.rs", paths, payload)
            self.assertNotIn("src/rename.rs", paths, payload)
            self.assertEqual(paths.count("src/renamed.rs"), 1, payload)
            self.assertNotIn(
                "src/changed.rs",
                paths,
                "the low-score Overlay upsert is outside Overlay top-k, but its "
                f"complete shadow must still hide the high-score Base row: {payload}",
            )
            self.assertNotIn("MARK_CHANGED_BASE", json.dumps(payload))

    def test_v2_view_search_progressively_backfills_shadowed_base_top_k(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, db_root, model = self._build_v2_view_fixture(
                Path(tmp),
                {
                    "src/deleted.rs": "//! MARK_BACKFILL_DELETED alpha\n",
                    "src/changed.rs": "//! MARK_BACKFILL_OLD alpha\n",
                    "src/keep_a.rs": "//! MARK_BACKFILL_A alpha\n",
                    "src/keep_b.rs": "//! MARK_BACKFILL_B alpha\n",
                    "src/keep_c.rs": "//! MARK_BACKFILL_C alpha\n",
                },
                {
                    "src/deleted.rs": None,
                    "src/changed.rs": "//! MARK_BACKFILL_NEW alpha\n",
                    "src/overlay.rs": "//! MARK_BACKFILL_OVERLAY alpha\n",
                },
                {
                    "MARK_BACKFILL_DELETED": 0.001,
                    "MARK_BACKFILL_OLD": 0.002,
                    "MARK_BACKFILL_OVERLAY": 0.0025,
                    "MARK_BACKFILL_A": 0.003,
                    "MARK_BACKFILL_B": 0.004,
                    "MARK_BACKFILL_C": 0.005,
                    "MARK_BACKFILL_NEW": 0.9,
                },
            )
            fetch_requests_by_collection = {}
            collection_counts = {}
            real_search = runner._search_collection_v2

            def record_fetch(collection, query, n_results, query_embedding=None):
                collection_identity = str(collection.id)
                fetch_requests_by_collection.setdefault(collection_identity, []).append(
                    n_results
                )
                collection_counts.setdefault(collection_identity, collection.count())
                return real_search(
                    collection,
                    query,
                    n_results,
                    query_embedding=query_embedding,
                )

            with mock.patch.object(
                runner, "_search_collection_v2", side_effect=record_fetch
            ):
                payload = self._search_v2_view(
                    project, db_root, model, n_results=3
                )

            self.assertTrue(payload.get("ok"), payload)
            self.assertEqual(
                [item["path"] for item in self._scope_file_results(payload, "files")],
                ["src/overlay.rs", "src/keep_a.rs", "src/keep_b.rs"],
                payload,
            )
            self.assertTrue(fetch_requests_by_collection)
            self.assertTrue(
                all(requests[0] == 3 for requests in fetch_requests_by_collection.values()),
                "every queried generation must start at requested top-k, never "
                f"eagerly scan its full collection: {fetch_requests_by_collection}",
            )
            base_collections = [
                identity
                for identity, count in collection_counts.items()
                if count == 5
            ]
            self.assertEqual(
                len(base_collections),
                1,
                f"fixture must identify exactly one five-row Base: {collection_counts}",
            )
            base_requests = fetch_requests_by_collection[base_collections[0]]
            self.assertEqual(base_requests[0], 3, base_requests)
            self.assertTrue(
                any(request > 3 for request in base_requests[1:]),
                "the same Base collection must be progressively re-queried after "
                f"its shadowed top-k, not fetched eagerly once: {base_requests}",
            )
            self.assertTrue(
                all(
                    request <= 7
                    for requests in fetch_requests_by_collection.values()
                    for request in requests
                ),
                "backfill must not exceed this fixture's seven physical rows: "
                f"{fetch_requests_by_collection}",
            )
            self.assertLessEqual(
                sum(len(requests) for requests in fetch_requests_by_collection.values()),
                8,
                f"backfill must stay bounded: {fetch_requests_by_collection}",
            )

    def test_v2_view_search_applies_all_terms_after_authoritative_union(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, db_root, model = self._build_v2_view_fixture(
                Path(tmp),
                {
                    "src/changed.rs": "//! MARK_TERMS_BASE alpha beta\n",
                    "src/strict.rs": "//! MARK_TERMS_STRICT alpha beta\n",
                },
                {"src/changed.rs": "//! MARK_TERMS_OVERLAY alpha only\n"},
                {
                    "MARK_TERMS_BASE": 0.01,
                    "MARK_TERMS_OVERLAY": 0.02,
                    "MARK_TERMS_STRICT": 0.3,
                },
            )

            payload = self._search_v2_view(
                project,
                db_root,
                model,
                query="alpha beta",
                match_mode="all_terms",
            )

            self.assertTrue(payload.get("ok"), payload)
            strict = self._scope_file_results(payload, "files")
            suggestions = list(
                ((payload.get("scope_results") or {}).get("files") or {}).get(
                    "suggestions"
                )
                or []
            )
            self.assertEqual([item["path"] for item in strict], ["src/strict.rs"])
            self.assertEqual(
                [item["path"] for item in suggestions], ["src/changed.rs"], payload
            )
            self.assertEqual(suggestions[0]["matched_terms"], ["alpha"])
            self.assertEqual(suggestions[0]["missing_terms"], ["beta"])
            self.assertIn("MARK_TERMS_OVERLAY", suggestions[0]["description"])
            self.assertNotIn("MARK_TERMS_BASE", suggestions[0]["description"])

    def test_v2_view_search_ranks_by_raw_distance_then_rounds_wire_value(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, db_root, model = self._build_v2_view_fixture(
                Path(tmp),
                {"src/a_base.rs": "//! MARK_RAW_BASE alpha\n"},
                {"src/z_overlay.rs": "//! MARK_RAW_OVERLAY alpha\n"},
                {
                    "MARK_RAW_BASE": 0.12341,
                    "MARK_RAW_OVERLAY": 0.12344,
                },
            )

            payload = self._search_v2_view(
                project, db_root, model, n_results=1
            )

            self.assertTrue(payload.get("ok"), payload)
            results = self._scope_file_results(payload, "files")
            self.assertEqual([item["path"] for item in results], ["src/a_base.rs"])
            self.assertEqual(results[0]["distance"], 0.1234)

    def test_v2_view_search_moves_scope_once_and_encodes_common_view_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, db_root, model = self._build_v2_view_fixture(
                Path(tmp),
                {
                    "src/move.rs": "//! MARK_SCOPE_OLD alpha\n",
                    "docs/guide.md": "# MARK_SCOPE_GUIDE alpha\n",
                },
                {
                    "src/move.rs": None,
                    "docs/move.md": "# MARK_SCOPE_NEW alpha\n",
                },
                {
                    "MARK_SCOPE_OLD": 0.01,
                    "MARK_SCOPE_NEW": 0.02,
                    "MARK_SCOPE_GUIDE": 0.3,
                },
            )

            payload = self._search_v2_view(
                project,
                db_root,
                model,
                scopes=("files", "files-docs"),
            )

            self.assertTrue(payload.get("ok"), payload)
            files = self._scope_file_results(payload, "files")
            docs = self._scope_file_results(payload, "files-docs")
            all_paths = [item["path"] for item in files + docs]
            self.assertEqual(model.query_encode_count, 1, payload)
            self.assertNotIn("src/move.rs", all_paths, payload)
            self.assertEqual(all_paths.count("docs/move.md"), 1, payload)
            moved = next(item for item in docs if item["path"] == "docs/move.md")
            self.assertIn("MARK_SCOPE_NEW", moved["description"])
            self.assertNotIn("MARK_SCOPE_OLD", json.dumps(payload))

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

    def test_search_multi_dispatch_forwards_explicit_file_index_protocol(self):
        args = argparse.Namespace(
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            db_root="",
            project_root="/fixture/project",
            query="alpha beta",
            n_results=3,
            scopes="issues,files",
            match_mode="all_terms",
            file_index_protocol="v2",
        )
        with mock.patch.object(
            runner,
            "action_search_multi_v2",
            return_value={"ok": True, "scope_results": {}},
        ) as search, mock.patch.object(runner, "emit") as emit:
            exit_code = runner._dispatch_v2("search-multi", args)

        self.assertEqual(exit_code, 0)
        search.assert_called_once_with(
            repo_hash=REPO_HASH,
            worktree_hash=WORKTREE_HASH,
            project_root="/fixture/project",
            query="alpha beta",
            n_results=3,
            scopes=["issues", "files"],
            match_mode="all_terms",
            db_root=None,
            file_index_protocol="v2",
        )
        emit.assert_called_once_with({"ok": True, "scope_results": {}})

    def test_search_multi_accepts_real_model_numpy_float32_query_embedding(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = self._seed_file_scopes(base, db_root)
            fake_model = runner._FakeEmbeddingModel()
            numpy_model = mock.MagicMock()
            numpy_model.encode.side_effect = lambda values: numpy.asarray(
                fake_model.encode(values), dtype=numpy.float32
            )

            with mock.patch.object(
                runner, "_get_embedding_model", return_value=numpy_model
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

            self.assertTrue(
                payload.get("ok"),
                "a healthy store must remain searchable when the real E5 model "
                f"returns NumPy float32 scalars: {payload}",
            )
            self.assertEqual(payload["scopes"]["files"]["state"], "fresh")
            self.assertEqual(payload["scopes"]["files-docs"]["state"], "fresh")

    def test_search_multi_reports_healthy_query_failure_without_corrupt_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = self._seed_file_scopes(base, db_root)

            with mock.patch.object(
                runner,
                "_search_scope_collection",
                side_effect=TypeError(
                    "private-vector=[0.123456789," + ("9" * 2048) + "]"
                ),
            ):
                payload = runner.action_search_multi_v2(
                    repo_hash=REPO_HASH,
                    worktree_hash=WORKTREE_HASH,
                    project_root=str(project),
                    query="alpha search",
                    n_results=5,
                    scopes=["files"],
                    db_root=db_root,
                )

            self.assertFalse(payload.get("ok"), payload)
            self.assertEqual(payload.get("error_code"), "SEARCH_FAILED", payload)
            self.assertIs(payload.get("retryable"), False, payload)
            self.assertEqual(payload.get("affected_scopes"), ["files"], payload)
            self.assertLessEqual(len((payload.get("error") or "").encode()), 512)
            self.assertNotIn("private-vector", json.dumps(payload))
            self.assertNotIn("scopes", payload)
            self.assertNotIn("scope_results", payload)

    def test_search_multi_reports_query_encoding_failure_without_runtime_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = self._seed_file_scopes(base, db_root)

            with mock.patch.object(
                runner.E5EmbeddingFunction,
                "embed_query",
                side_effect=ValueError("model emitted a non-finite embedding"),
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

            self.assertFalse(payload.get("ok"), payload)
            self.assertEqual(payload.get("error_code"), "SEARCH_FAILED", payload)
            self.assertIs(payload.get("retryable"), False, payload)
            self.assertEqual(
                payload.get("affected_scopes"), ["files", "files-docs"], payload
            )
            self.assertLessEqual(len((payload.get("error") or "").encode()), 512)
            self.assertNotIn("non-finite", json.dumps(payload))

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

    def test_v2_view_verifier_rejects_checksum_descriptor_manifest_and_count_mismatch(self):
        """FR-409/FR-412: prepublish closure verification rejects mixed pairs.

        This deliberately stops at view selection. Overlay/base query merge,
        ranking, and bounded backfill are owned by T-IDX-428; active/previous/
        legacy fallback and classification are owned by T-IDX-430.
        """
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            db_root = base / "index"
            project = base / "project"
            (project / "src").mkdir(parents=True)
            (project / "docs").mkdir()
            (project / "src" / "alpha.rs").write_text(
                "//! alpha implementation\nfn alpha() {}\n", encoding="utf-8"
            )
            (project / "docs" / "guide.md").write_text(
                "# Alpha guide\n", encoding="utf-8"
            )
            git_env = os.environ.copy()
            git_env["GIT_CONFIG_NOSYSTEM"] = "1"
            git_env["GIT_CONFIG_GLOBAL"] = os.devnull
            git_env["GIT_ATTR_NOSYSTEM"] = "1"
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
                git_env.pop(key, None)
            for key in list(git_env):
                if (
                    key == "GIT_CONFIG_COUNT"
                    or key.startswith("GIT_CONFIG_KEY_")
                    or key.startswith("GIT_CONFIG_VALUE_")
                ):
                    git_env.pop(key)
            git_commands = (
                ["git", "init", "--quiet", str(project)],
                [
                    "git",
                    "-C",
                    str(project),
                    "symbolic-ref",
                    "HEAD",
                    "refs/heads/develop",
                ],
                ["git", "-C", str(project), "add", "."],
                [
                    "git",
                    "-C",
                    str(project),
                    "-c",
                    "user.name=gwt tests",
                    "-c",
                    "user.email=gwt-tests@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "canonical view fixture",
                ],
            )
            for command in git_commands:
                try:
                    completed = subprocess.run(
                        command,
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
                    f"{' '.join(command)} failed\n{completed.stderr}",
                )
            result = runner.action_index_files_v2(
                project_root=str(project),
                repo_hash=REPO_HASH,
                worktree_hash=WORKTREE_HASH,
                mode="full",
                db_root=db_root,
                scope="files",
                file_index_protocol="v2",
            )
            self.assertTrue(result.get("ok"), result)

            v2_root = runner.resolve_file_index_v2_root(REPO_HASH, db_root=db_root)
            worktree_root = v2_root / "worktrees" / WORKTREE_HASH
            head_path = worktree_root / "head.json"
            self.assertTrue(head_path.is_file(), "v2 build must publish a view head")
            head = json.loads(head_path.read_text(encoding="utf-8"))
            view_id = head.get("active_view_id")
            self.assertIsInstance(view_id, str, head)
            head_payload = {
                "schema_version": head["schema_version"],
                "active_view_id": head["active_view_id"],
                "previous_view_id": head["previous_view_id"],
                "sequence": head["sequence"],
            }
            expected_head_checksum = runner._sha256_json(head_payload)
            self.assertEqual(head.get("checksum"), expected_head_checksum)
            view_dir = worktree_root / "views" / view_id
            view_descriptor_path = view_dir / "descriptor.json"
            view = json.loads(view_descriptor_path.read_text(encoding="utf-8"))
            self.assertRegex(view.get("descriptor_checksum") or "", r"^[0-9a-f]{64}$")
            base_dir = runner.resolve_file_index_v2_base_dir(
                REPO_HASH, view["base_generation_id"], db_root=db_root
            )
            overlay_dir = runner.resolve_file_index_v2_overlay_dir(
                REPO_HASH,
                WORKTREE_HASH,
                view["overlay_generation_id"],
                db_root=db_root,
            )
            verify = getattr(runner, "_verify_file_index_v2_view", None)
            self.assertTrue(
                callable(verify),
                "T-IDX-427 requires a prepublish Worktree View closure verifier",
            )
            self.assertTrue(
                verify(view_dir),
                "the freshly published active view must verify before mutation",
            )

            mutation_cases = {
                "view descriptor checksum": (
                    view_descriptor_path,
                    lambda payload: payload.update(
                        {"overlay_generation_id": "different-overlay"}
                    ),
                ),
                "overlay descriptor pair": (
                    overlay_dir / "descriptor.json",
                    lambda payload: payload.update(
                        {"base_generation_id": "different-base"}
                    ),
                ),
                "base manifest digest": (
                    base_dir / "manifest.json",
                    lambda payload: payload["entries"].append(
                        dict(payload["entries"][0], path="src/phantom.rs")
                    ),
                ),
                "overlay manifest digest": (
                    overlay_dir / "manifest.json",
                    lambda payload: payload["entries"].append(
                        {"path": "src/overlay-phantom.rs"}
                    ),
                ),
                "base physical count": (
                    base_dir / "descriptor.json",
                    lambda payload: payload["document_counts"].update(
                        {"files": payload["document_counts"]["files"] + 1}
                    ),
                ),
            }
            for label, (path, mutate) in mutation_cases.items():
                with self.subTest(label=label):
                    original = path.read_bytes()
                    try:
                        payload = json.loads(original.decode("utf-8"))
                        mutate(payload)
                        path.write_text(
                            json.dumps(
                                payload,
                                sort_keys=True,
                                separators=(",", ":"),
                                ensure_ascii=True,
                            ),
                            encoding="utf-8",
                        )
                        self.assertFalse(
                            verify(view_dir),
                            f"{label} mismatch must reject the complete view",
                        )
                    finally:
                        path.write_bytes(original)

            base_manifest_path = base_dir / "manifest.json"
            base_descriptor_path = base_dir / "descriptor.json"
            original_manifest = base_manifest_path.read_bytes()
            original_descriptor = base_descriptor_path.read_bytes()

            def write_coherent_manifest_pair(manifest: dict, descriptor: dict) -> None:
                descriptor["manifest_digest"] = runner._sha256_json(
                    manifest["entries"]
                )
                base_manifest_path.write_text(
                    json.dumps(
                        manifest,
                        sort_keys=True,
                        separators=(",", ":"),
                        ensure_ascii=True,
                    ),
                    encoding="utf-8",
                )
                base_descriptor_path.write_text(
                    json.dumps(
                        descriptor,
                        sort_keys=True,
                        separators=(",", ":"),
                        ensure_ascii=True,
                    ),
                    encoding="utf-8",
                )

            for label, field, replacement in (
                (
                    "content-addressed generation id",
                    "cas_key",
                    "c" * 64,
                ),
                (
                    "visible source snapshot id",
                    "source_digest",
                    "d" * 64,
                ),
            ):
                with self.subTest(label=label):
                    try:
                        manifest = json.loads(original_manifest.decode("utf-8"))
                        descriptor = json.loads(original_descriptor.decode("utf-8"))
                        manifest["entries"][0][field] = replacement
                        write_coherent_manifest_pair(manifest, descriptor)
                        self.assertFalse(
                            verify(view_dir),
                            f"{label} must be recomputed from the canonical manifest",
                        )
                    finally:
                        base_manifest_path.write_bytes(original_manifest)
                        base_descriptor_path.write_bytes(original_descriptor)


if __name__ == "__main__":
    unittest.main()
