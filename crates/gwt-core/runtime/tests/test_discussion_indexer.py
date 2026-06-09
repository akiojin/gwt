"""Tests for the discussions semantic index scope.

Discussion entries live in `.gwt/work/discussions.md` as H2 sections with the
canonical shape:

    ## YYYY-MM-DD — title
    Status: active
    Topics: workspace, work
    Related SPECs: #2359

    Summary:
    ...
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


SAMPLE_DISCUSSIONS = """# Discussions

## 2026-05-22 — Workspace terminology

Status: active
Topics: workspace, work, discussion
Related SPECs: #2359
Related Works:
Promoted To:

Summary:
Workspace is being split into Project State, Work, Agent, Discussion, and Branch.

Decisions:
- Discussion is not Work.
- Work is durable.

Open Questions:
- Topic Stack persistence.

Next:
Define Project State migration.

## 2026-05-21 — Agent title labels

Status: completed
Topics: agent, title
Related SPECs: #2359

Summary:
Agent role badges should show Codex or Claude Code.
"""


class LoadDiscussionDocumentsTests(unittest.TestCase):
    def _write_discussions_file(self, contents: str) -> Path:
        root = Path(tempfile.mkdtemp())
        work_dir = root / ".gwt" / "work"
        work_dir.mkdir(parents=True, exist_ok=True)
        (work_dir / "discussions.md").write_text(contents, encoding="utf-8")
        return root

    def test_returns_chunks_for_each_h2_section(self):
        root = self._write_discussions_file(SAMPLE_DISCUSSIONS)
        discussions, manifest = runner._load_discussion_documents(str(root))

        self.assertEqual(len(discussions), 2)
        self.assertEqual(len(manifest), 1)
        self.assertEqual(manifest[0]["path"], ".gwt/work/discussions.md")

    def test_extracts_status_topics_and_related_specs(self):
        root = self._write_discussions_file(SAMPLE_DISCUSSIONS)
        discussions, _manifest = runner._load_discussion_documents(str(root))
        first = discussions[0]

        self.assertEqual(first["date"], "2026-05-22")
        self.assertEqual(first["title"], "Workspace terminology")
        self.assertEqual(first["status"], "active")
        self.assertEqual(first["topics"], ["workspace", "work", "discussion"])
        self.assertEqual(first["related_specs"], ["2359"])
        self.assertEqual(first["related_works"], [])
        self.assertEqual(first["promoted_to"], [])


class BuildDiscussionRecordsTests(unittest.TestCase):
    def test_returns_chroma_records_with_metadata(self):
        discussions = [
            {
                "discussion_id": "abc123def456",
                "date": "2026-05-22",
                "title": "Workspace terminology",
                "status": "active",
                "topics": ["workspace", "work"],
                "related_specs": ["2359"],
                "heading": "## 2026-05-22 — Workspace terminology",
                "body": "Summary:\nWorkspace is being split.",
                "chunk_idx": 0,
                "total_chunks": 1,
            }
        ]

        records = runner._build_discussion_records(discussions)

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["id"], "discussion-abc123def456")
        self.assertIn("Workspace terminology", records[0]["document"])
        meta = records[0]["metadata"]
        self.assertEqual(meta["status"], "active")
        self.assertEqual(meta["topics"], "workspace,work")
        self.assertEqual(meta["related_specs"], "2359")


class ActionIndexDiscussionsTests(unittest.TestCase):
    def test_full_mode_writes_manifest_and_chunks(self):
        with tempfile.TemporaryDirectory() as wt, tempfile.TemporaryDirectory() as db_root_dir:
            root = Path(wt)
            work_dir = root / ".gwt" / "work"
            work_dir.mkdir(parents=True, exist_ok=True)
            (work_dir / "discussions.md").write_text(SAMPLE_DISCUSSIONS, encoding="utf-8")
            collection = _FakeCollection()

            with mock.patch.object(
                runner,
                "_make_chroma_collection_repairing",
                return_value=(_FakeClient(), collection),
            ), mock.patch.object(runner, "_close_chroma_client"):
                result = runner.action_index_discussions_v2(
                    project_root=str(root),
                    repo_hash="abc1234567890def",
                    worktree_hash=None,
                    mode="full",
                    db_root=Path(db_root_dir),
                )

            self.assertTrue(result.get("ok"), result)
            self.assertEqual(result["scope"], "discussions")
            self.assertGreaterEqual(result["indexed"], 2)
            self.assertEqual(len(collection.upserts), 1)

            db_path = runner.resolve_db_path(
                "abc1234567890def", None, "discussions", db_root=Path(db_root_dir)
            )
            manifest_file = runner._manifest_path(db_path, "discussions")
            self.assertTrue(manifest_file.is_file(), f"missing manifest at {manifest_file}")
            manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
            entries = manifest.get("entries") if isinstance(manifest, dict) else manifest
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["path"], ".gwt/work/discussions.md")


class _FakeClient:
    pass


class _FakeCollection:
    def __init__(self) -> None:
        self.ids = []
        self.upserts = []

    def get(self):
        return {"ids": list(self.ids)}

    def delete(self, ids):
        self.ids = [existing for existing in self.ids if existing not in ids]

    def upsert(self, ids, documents, metadatas):
        self.ids.extend(ids)
        self.upserts.append(
            {
                "ids": ids,
                "documents": documents,
                "metadatas": metadatas,
            }
        )


if __name__ == "__main__":
    unittest.main()
