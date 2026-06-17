"""SPEC-2359 US-80: Work history (`works`) semantic index scope.

Past Work items — including completed and discarded ones — become searchable so
work done weeks ago can be rediscovered at Start Work time. The primary on-disk
source is the home projection JSON
(`~/.gwt/projects/<repo_hash>/project-state/works.json`); when absent, the
runner folds the event logs (`work-events.jsonl` / repo-local
`.gwt/work/events.jsonl`) into items.
"""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class LoadWorkDocumentsTests(unittest.TestCase):
    REPO_HASH = "abc1234567890def"

    def _state_dir(self, home: Path) -> Path:
        state = home / ".gwt" / "projects" / self.REPO_HASH / "project-state"
        state.mkdir(parents=True, exist_ok=True)
        return state

    def _write_works_json(self, home: Path) -> None:
        state = self._state_dir(home)
        projection = {
            "updated_at": "2026-05-20T00:00:00Z",
            "work_items": [
                {
                    "id": "work-active",
                    "title": "Active project tab work",
                    "intent": "Finalize PR review thread cleanup.",
                    "summary": "develop 取り込み後の検証",
                    "status_category": "active",
                    "owner": "SPEC-2359",
                    "execution_containers": [
                        {
                            "branch": "work/20260511-0113",
                            "worktree_path": "/tmp/wt1",
                            "pr_number": 2641,
                            "pr_url": "https://github.com/akiojin/gwt/pull/2641",
                            "pr_state": "OPEN",
                        }
                    ],
                    "board_refs": ["board-xyz"],
                    "related_work_item_ids": ["work-old"],
                    "discarded": False,
                    "events": [],
                },
                {
                    "id": "work-done",
                    "title": "Completed migration work",
                    "intent": "Migrate the index layout.",
                    "summary": "",
                    "status_category": "done",
                    "owner": None,
                    "execution_containers": [],
                    "board_refs": [],
                    "related_work_item_ids": [],
                    "discarded": False,
                    "events": [],
                },
                {
                    "id": "work-discarded",
                    "title": "Abandoned spike",
                    "intent": "Spike that was discarded.",
                    "summary": "",
                    "status_category": "idle",
                    "owner": None,
                    "execution_containers": [],
                    "board_refs": [],
                    "related_work_item_ids": [],
                    "discarded": True,
                    "events": [],
                },
            ],
        }
        (state / "works.json").write_text(
            json.dumps(projection), encoding="utf-8"
        )

    def test_loads_all_works_including_terminal_from_projection(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            self._write_works_json(home)
            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                works, manifest = runner._load_work_documents(
                    self.REPO_HASH, project_root=None
                )
            ids = [w["id"] for w in works]
            # All three are returned: active, done, AND discarded.
            self.assertEqual(
                sorted(ids), ["work-active", "work-discarded", "work-done"]
            )
            self.assertEqual(len(manifest), 1)
            self.assertEqual(manifest[0]["path"], "project-state/works.json")

    def test_falls_back_to_folding_event_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            state = self._state_dir(home)
            events = [
                {
                    "id": "ev-1",
                    "work_item_id": "ws-fold",
                    "kind": "update",
                    "title": "Folded work title",
                    "intent": "Folded intent",
                    "summary": None,
                    "status_category": "active",
                    "owner": "owner-a",
                    "execution_container": {
                        "branch": "feature/fold",
                        "pr_number": 99,
                        "pr_url": "https://example/pull/99",
                    },
                    "updated_at": "2026-05-20T00:00:00Z",
                },
                {
                    "id": "ev-2",
                    "work_item_id": "ws-fold",
                    "kind": "done",
                    "title": None,
                    "intent": None,
                    "summary": "wrapped up",
                    "status_category": None,
                    "updated_at": "2026-05-20T01:00:00Z",
                },
            ]
            (state / "work-events.jsonl").write_text(
                "\n".join(json.dumps(e) for e in events) + "\n",
                encoding="utf-8",
            )
            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                works, manifest = runner._load_work_documents(
                    self.REPO_HASH, project_root=None
                )
            self.assertEqual(len(works), 1)
            work = works[0]
            self.assertEqual(work["id"], "ws-fold")
            self.assertEqual(work["title"], "Folded work title")
            self.assertEqual(work["intent"], "Folded intent")
            self.assertEqual(work["summary"], "wrapped up")
            # Latest event was kind=done with no explicit status_category.
            self.assertEqual(work["status_category"], "done")
            self.assertTrue(manifest)


class BuildWorkRecordsTests(unittest.TestCase):
    SAMPLE_WORK = {
        "id": "work-1",
        "title": "Title here",
        "intent": "Intent here",
        "summary": "Summary here",
        "status_category": "active",
        "owner": "SPEC-2359",
        "execution_containers": [
            {
                "branch": "work/20260101-0000",
                "pr_number": 1234,
                "pr_url": "https://github.com/akiojin/gwt/pull/1234",
            }
        ],
        "board_refs": ["board-aaa"],
        "related_work_item_ids": ["work-prev"],
        "discarded": False,
    }

    def test_document_joins_searchable_fields(self):
        doc = runner._work_entry_document(self.SAMPLE_WORK)
        for needle in (
            "Title here",
            "Intent here",
            "Summary here",
            "SPEC-2359",
            "work/20260101-0000",
            "#1234",
            "https://github.com/akiojin/gwt/pull/1234",
            "board-aaa",
            "work-prev",
        ):
            self.assertIn(needle, doc)

    def test_record_metadata_carries_contract_fields(self):
        records = runner._build_work_records([self.SAMPLE_WORK])
        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["id"], "work-work-1")
        meta = record["metadata"]
        # Contract fields the Rust work_result() reads.
        self.assertEqual(meta["work_id"], "work-1")
        self.assertEqual(meta["title"], "Title here")
        self.assertEqual(meta["intent"], "Intent here")
        self.assertEqual(meta["status"], "active")
        self.assertEqual(meta["branches"], "work/20260101-0000")
        self.assertEqual(meta["pr_numbers"], "1234")

    def test_discarded_status_is_surfaced(self):
        work = dict(self.SAMPLE_WORK)
        work["discarded"] = True
        records = runner._build_work_records([work])
        self.assertEqual(records[0]["metadata"]["status"], "discarded")

    def test_records_without_id_are_dropped(self):
        records = runner._build_work_records([{"id": "", "title": "no id"}])
        self.assertEqual(records, [])


class FormatWorkResultsTests(unittest.TestCase):
    def test_emits_contract_fields_and_drops_missing_work_id(self):
        items = [
            {
                "id": "work-good",
                "metadata": {
                    "work_id": "good",
                    "title": "Good work",
                    "intent": "good intent",
                    "status": "done",
                    "owner": "me",
                    "branches": "a,b",
                    "pr_numbers": "5,6",
                },
                "distance": 0.12,
            },
            {
                "id": "work-bad",
                "metadata": {"work_id": "", "title": "Bad"},
                "distance": 0.5,
            },
        ]
        formatted = runner._format_work_results(items)
        self.assertEqual(len(formatted), 1)
        result = formatted[0]
        self.assertEqual(result["work_id"], "good")
        self.assertEqual(result["title"], "Good work")
        self.assertEqual(result["intent"], "good intent")
        self.assertEqual(result["status"], "done")
        self.assertEqual(result["branches"], ["a", "b"])
        self.assertEqual(result["pr_numbers"], ["5", "6"])
        self.assertEqual(result["distance"], 0.12)


class WorksStatusTests(unittest.TestCase):
    REPO_HASH = "abc1234567890def"

    def test_status_includes_works_scope(self):
        with tempfile.TemporaryDirectory() as tmp:
            db_root = Path(tmp) / "index_root"
            with mock.patch.object(
                runner, "_scope_document_count", return_value=(False, 0)
            ):
                status = runner.action_status_v2(
                    repo_hash=self.REPO_HASH,
                    worktree_hash=None,
                    db_root=db_root,
                )
            self.assertIn("works", status["status"])
            self.assertEqual(
                status["status"]["works"]["reason"], "collection_missing"
            )


class WorksIndexRoundtripTests(unittest.TestCase):
    REPO_HASH = "abc1234567890def"

    def test_index_then_search_returns_work_results(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            db_root = home / "index_root"
            state = home / ".gwt" / "projects" / self.REPO_HASH / "project-state"
            state.mkdir(parents=True, exist_ok=True)
            projection = {
                "updated_at": "2026-05-20T00:00:00Z",
                "work_items": [
                    {
                        "id": "work-search-me",
                        "title": "Searchable migration work",
                        "intent": "Index layout migration was completed here.",
                        "summary": "",
                        "status_category": "done",
                        "owner": "SPEC-2359",
                        "execution_containers": [],
                        "board_refs": [],
                        "related_work_item_ids": [],
                        "discarded": False,
                        "events": [],
                    }
                ],
            }
            (state / "works.json").write_text(json.dumps(projection), encoding="utf-8")

            with mock.patch.dict(os.environ, {"HOME": str(home)}, clear=False):
                build = runner.action_index_works_v2(
                    project_root=None,
                    repo_hash=self.REPO_HASH,
                    worktree_hash=None,
                    mode="full",
                    db_root=db_root,
                )
                self.assertTrue(build["ok"])
                self.assertEqual(build["indexed"], 1)

                result = runner.action_search_v2(
                    action="search-works",
                    repo_hash=self.REPO_HASH,
                    worktree_hash=None,
                    project_root=None,
                    query="migration",
                    n_results=10,
                    no_auto_build=True,
                    db_root=db_root,
                )
            self.assertTrue(result["ok"])
            self.assertIn("workResults", result)
            work_ids = [item["work_id"] for item in result["workResults"]]
            self.assertIn("work-search-me", work_ids)


if __name__ == "__main__":
    unittest.main()
