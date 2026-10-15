"""SPEC-1939: Board semantic index scope."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class BoardIndexerTests(unittest.TestCase):
    REPO_HASH = "abc1234567890def"

    def test_background_incremental_yields_after_one_record_and_resumes(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            self._write_board_history(home)
            db_root = home / "index"
            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                with mock.patch.object(runner, "_pending_higher_priority", return_value=True):
                    partial = runner.action_index_board_v2(
                        self.REPO_HASH, None, mode="incremental", db_root=db_root,
                        qos="background",
                    )
                self.assertTrue(partial["yielded"], partial)
                self.assertTrue(partial["resumable"], partial)
                self.assertEqual(partial["indexed"], 1)
                db_path = runner.resolve_db_path(self.REPO_HASH, None, "board", db_root)
                self.assertFalse(runner._manifest_path(db_path, "board").exists())
                self.assertEqual(runner._scope_document_count(db_path, "board"), (True, 1))
                resumed = runner.action_index_board_v2(
                    self.REPO_HASH, None, mode="incremental", db_root=db_root,
                    qos="background",
                )
                self.assertTrue(resumed["ok"], resumed)
                self.assertEqual(resumed["indexed"], 1)
                self.assertEqual(resumed["document_count"], 2)
                self.assertEqual(runner._read_scope_meta(self.REPO_HASH, None, "board", db_root)["document_count"], 2)

    def test_incremental_noop_keeps_document_count_and_updates_only_changed_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            self._write_board_history(home)
            db_root = home / "index"
            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                built = runner.action_index_board_v2(self.REPO_HASH, None, db_root=db_root)
                self.assertTrue(built["ok"], built)
                db_path = runner.resolve_db_path(self.REPO_HASH, None, "board", db_root)
                client, collection = runner._make_chroma_collection(runner.resolve_active_store(db_path), runner.V2_BOARD_COLLECTION)
                try:
                    recording = mock.Mock(wraps=collection)
                    with mock.patch.object(runner, "_make_chroma_collection", return_value=(client, recording)), mock.patch.object(runner, "_close_chroma_client"):
                        noop = runner.action_index_board_v2(self.REPO_HASH, None, mode="incremental", db_root=db_root)
                        self.assertEqual(noop["indexed"], 0)
                        meta = runner._read_scope_meta(self.REPO_HASH, None, "board", db_root)
                        self.assertEqual(meta["document_count"], 2)
                        self.assertEqual(meta["source_document_count"], 2)
                        recording.upsert.assert_not_called()
                        segment = home / ".gwt" / "projects" / self.REPO_HASH / "coordination" / "events" / "000001.jsonl"
                        events = [json.loads(line) for line in segment.read_text(encoding="utf-8").splitlines()]
                        events[0]["entry"]["body"] = "Changed Board document"
                        segment.write_text("\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8")
                        changed = runner.action_index_board_v2(self.REPO_HASH, None, mode="incremental", db_root=db_root)
                        self.assertEqual(changed["indexed"], 1)
                        self.assertEqual(recording.upsert.call_args.kwargs["ids"], ["board-1"])
                        recording.delete.assert_not_called()
                        segment.write_text(json.dumps(events[0]) + "\n", encoding="utf-8")
                        removed = runner.action_index_board_v2(self.REPO_HASH, None, mode="incremental", db_root=db_root)
                        self.assertTrue(removed["ok"], removed)
                        self.assertEqual(recording.delete.call_args.kwargs["ids"], ["board-2"])
                        self.assertEqual(collection.count(), 1)
                finally:
                    client.close()

    def test_full_build_with_dropped_upserts_does_not_publish_or_advance_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            self._write_board_history(home)
            db_root = home / "index"
            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                collection = mock.Mock()
                collection.get.return_value = {"ids": []}
                collection.count.return_value = 0
                with mock.patch.object(runner, "_make_chroma_collection_repairing", return_value=(mock.Mock(), collection)), mock.patch.object(runner, "_open_chroma_collection", return_value=(mock.Mock(), collection)):
                    result = runner.action_index_board_v2(self.REPO_HASH, None, db_root=db_root)
                self.assertFalse(result["ok"], result)
                self.assertEqual(result["error_code"], "BUILD_VERIFY_FAILED")
                db_path = runner.resolve_db_path(self.REPO_HASH, None, "board", db_root)
                self.assertFalse(runner.active_pointer_path(db_path).exists())
                self.assertFalse(runner._manifest_path(db_path, "board").exists())
                self.assertEqual(runner._read_scope_meta(self.REPO_HASH, None, "board", db_root), {})

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


@unittest.skipUnless(
    os.name == "nt" and os.environ.get("GWT_INDEX_REAL_SOURCE_CHECK") == "1",
    "opt-in Windows check reads real source snapshots into an isolated index",
)
class RealRepoScopeCountTests(unittest.TestCase):
    def test_windows_runner_source_and_published_counts_match(self):
        repo_hash = os.environ["GWT_REPO_HASH"]
        project_root = os.environ["GWT_PROJECT_ROOT"]
        real_project = runner._gwt_home() / "projects" / repo_hash
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp) / "home"
            project = home / ".gwt" / "projects" / repo_hash
            # Freeze the real files so concurrent Board/Work posts cannot
            # change the count between the source read and the assertion.
            coordination = real_project / "coordination"
            manifest = coordination / "events.manifest.json"
            frozen_coordination = project / "coordination"
            (frozen_coordination / "events").mkdir(parents=True)
            manifest_bytes = manifest.read_bytes()
            (frozen_coordination / manifest.name).write_bytes(manifest_bytes)
            for segment in json.loads(manifest_bytes)["segments"]:
                shutil.copyfile(coordination / "events" / segment["file"], frozen_coordination / "events" / segment["file"])
            for relative in ("work-notes/discussions.md", "project-state/works.json"):
                destination = project / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(real_project / relative, destination)

            db_root = Path(tmp) / "index"
            env = dict(os.environ, HOME=str(home), USERPROFILE=str(home), GWT_INDEX_FAKE_EMBEDDING="1")
            evidence = []
            with mock.patch.dict(os.environ, env, clear=False):
                loaders = {
                    "board": lambda: runner._build_board_records(runner._load_board_documents(repo_hash, project_root)[0]),
                    "discussions": lambda: runner._build_discussion_records(runner._load_discussion_documents(project_root, repo_hash)[0]),
                    "works": lambda: runner._build_work_records(runner._load_work_documents(repo_hash, project_root)[0]),
                }
                for scope, load_records in loaders.items():
                    source_count = len({record["id"] for record in load_records()})
                    for mode in ("full", "incremental"):
                        started = time.perf_counter()
                        completed = subprocess.run(
                            [sys.executable, str(Path(runner.__file__).resolve()), "--action", f"index-{scope}", "--repo-hash", repo_hash, "--project-root", project_root, "--mode", mode, "--qos", "interactive", "--db-root", str(db_root)],
                            capture_output=True, text=True, env=env, timeout=180,
                        )
                        self.assertEqual(completed.returncode, 0, completed.stderr)
                        result = json.loads(completed.stdout)
                        self.assertTrue(result["ok"], result)
                        self.assertEqual(result["document_count"], source_count)
                        if mode == "incremental":
                            self.assertEqual(result["indexed"], 0, result)
                        count_ok, indexed_count = runner._scope_document_count(
                            runner.resolve_db_path(repo_hash, None, scope, db_root), scope
                        )
                        self.assertTrue(count_ok)
                        self.assertEqual(indexed_count, source_count)
                        pointer = runner._read_active_pointer(runner.resolve_db_path(repo_hash, None, scope, db_root))
                        self.assertEqual(pointer["document_count"], source_count)
                        meta = runner._read_scope_meta(repo_hash, None, scope, db_root)
                        self.assertEqual(meta["document_count"], source_count)
                        self.assertEqual(meta["source_document_count"], source_count)
                        evidence.append({"scope": scope, "mode": mode, "source": source_count, "collection": indexed_count, "pointer": pointer["document_count"], "meta": meta["document_count"], "seconds": round(time.perf_counter() - started, 3)})
            print(json.dumps({"platform": sys.platform, "fake_embedding": True, "counts": evidence}))
