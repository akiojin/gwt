"""SPEC-1939: Board semantic index scope."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class BoardIndexerTests(unittest.TestCase):
    REPO_HASH = "abc1234567890def"

    def _write_board_history(self, home: Path) -> None:
        coordination = home / ".gwt" / "projects" / self.REPO_HASH / "coordination"
        events = coordination / "events"
        events.mkdir(parents=True)
        segment = events / "000001.jsonl"
        entries = [
            {
                "id": "board-1",
                "author_kind": "agent",
                "author": "Codex",
                "kind": "status",
                "body": "Indexed content should include Board discussion history.",
                "title_summary": "Board search",
                "state": None,
                "parent_id": None,
                "created_at": "2026-05-20T00:00:00Z",
                "updated_at": "2026-05-20T00:00:00Z",
                "related_topics": ["index-search"],
                "related_owners": ["SPEC-1939"],
                "origin_branch": "work/index-search",
                "origin_session_id": "session-a",
                "origin_agent_id": "codex",
                "target_owners": [],
                "mentions": [],
                "audience": [],
            },
            {
                "id": "board-2",
                "author_kind": "agent",
                "author": "Claude Code",
                "kind": "decision",
                "body": "Workspace scoped Board search result.",
                "title_summary": "Workspace Board search",
                "state": None,
                "parent_id": None,
                "created_at": "2026-05-20T00:01:00Z",
                "updated_at": "2026-05-20T00:01:00Z",
                "related_topics": [],
                "related_owners": [],
                "origin_branch": "work/index-search",
                "origin_session_id": "session-b",
                "origin_agent_id": "claude",
                "target_owners": [],
                "mentions": [],
                "audience": ["workspace-a"],
            },
        ]
        segment.write_text(
            "\n".join(json.dumps({"type": "message_appended", "entry": entry}) for entry in entries)
            + "\n",
            encoding="utf-8",
        )
        (coordination / "events.manifest.json").write_text(
            json.dumps(
                {
                    "version": 1,
                    "active_segment": "000001.jsonl",
                    "segments": [
                        {
                            "file": "000001.jsonl",
                            "entries": 2,
                            "bytes": segment.stat().st_size,
                            "first_created_at": "2026-05-20T00:00:00Z",
                            "last_created_at": "2026-05-20T00:01:00Z",
                            "max_updated_at": "2026-05-20T00:01:00Z",
                            "first_entry_id": "board-1",
                            "last_entry_id": "board-2",
                        }
                    ],
                    "updated_at": "2026-05-20T00:01:00Z",
                }
            ),
            encoding="utf-8",
        )

    def test_board_documents_are_loaded_from_project_coordination_history(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            self._write_board_history(home)

            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                docs, manifest = runner._load_board_documents(self.REPO_HASH, project_root=None)

            self.assertEqual([doc["entry_id"] for doc in docs], ["board-1", "board-2"])
            self.assertEqual(docs[0]["title_summary"], "Board search")
            self.assertEqual(docs[1]["audience"], ["workspace-a"])
            self.assertEqual(manifest[0]["path"], "coordination/events/000001.jsonl")

    def test_status_includes_board_scope(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"

            with mock.patch.object(runner, "_scope_document_count", return_value=(False, 0)):
                status = runner.action_status_v2(
                    repo_hash=self.REPO_HASH,
                    worktree_hash=None,
                    db_root=db_root,
                )

            self.assertIn("board", status["status"])
            self.assertEqual(status["status"]["board"]["reason"], "collection_missing")
