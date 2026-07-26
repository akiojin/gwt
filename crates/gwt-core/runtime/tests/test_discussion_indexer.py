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

import hashlib
import json
import os
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

REPEATED_DISCUSSION_TODO = """# Discussions

## 2026-07-25 — Earlier proposal

Status: active

Summary:
An earlier proposal has the same nested heading and body.

## Discussion TODO

### Proposal A - Continue work [active]
- Next Question: Continue?

## 2026-07-26 — Discussion boundary

Status: active

Summary:
The proposal summary contains its own Discussion TODO.

## Discussion TODO

### Proposal A - Continue work [active]
- Next Question: Continue?

## Discussion TODO

### Proposal A - Continue work [active]
- Next Question: Continue?
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
        self.assertEqual(manifest[0]["path"], str(root / ".gwt" / "work" / "discussions.md"))

    def test_home_work_notes_file_wins_over_repo_local(self):
        # SPEC-3214 (FR-007): the machine-local home work-notes file is the
        # canonical source; the repo-local file is only a fallback.
        root = self._write_discussions_file(SAMPLE_DISCUSSIONS)
        with tempfile.TemporaryDirectory() as home:
            notes_dir = Path(home) / ".gwt" / "projects" / "hash1234" / "work-notes"
            notes_dir.mkdir(parents=True, exist_ok=True)
            home_discussions = (
                "# Discussions\n\n## 2026-07-03 — home-only discussion\n\n"
                "Status: active\nTopics: intake\n\nSummary:\nhome body.\n"
            )
            (notes_dir / "discussions.md").write_text(home_discussions, encoding="utf-8")
            previous_home = os.environ.get("HOME")
            os.environ["HOME"] = home
            try:
                discussions, manifest = runner._load_discussion_documents(str(root), "hash1234")
            finally:
                if previous_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = previous_home
        self.assertEqual(len(discussions), 1)
        self.assertEqual(discussions[0]["title"], "home-only discussion")
        self.assertEqual(manifest[0]["path"], str(notes_dir / "discussions.md"))

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
        old_digest = hashlib.sha1(
            f"{first['heading']}\n{first['body']}".encode("utf-8")
        ).hexdigest()[:12]
        first_record = runner._build_discussion_records([first])[0]
        self.assertEqual(first_record["id"], f"discussion-{old_digest}")

    def test_repeated_nested_h2_chunks_receive_unique_record_ids(self):
        root = self._write_discussions_file(REPEATED_DISCUSSION_TODO)

        discussions, _manifest = runner._load_discussion_documents(str(root))
        records = runner._build_discussion_records(discussions)
        record_ids = [record["id"] for record in records]

        self.assertEqual(len(record_ids), len(set(record_ids)), record_ids)
        todo_records = [
            record
            for record in records
            if record["metadata"]["title"] == "Discussion TODO"
        ]
        self.assertEqual(len(todo_records), 3)
        self.assertEqual(
            [record["metadata"]["chunk_idx"] for record in todo_records], [1, 1, 2]
        )
        self.assertEqual(
            [record["metadata"]["total_chunks"] for record in todo_records],
            [2, 3, 3],
        )

    def test_adding_another_proposal_does_not_renumber_existing_chunk_ids(self):
        existing = """## 2026-07-26 — Existing proposal

Status: active

Summary:
Existing body.

## Discussion TODO

### Proposal E - Existing [active]
- Next Question: Existing question?
"""
        prefix = """## 2026-07-25 — New earlier proposal

Status: active

Summary:
Earlier body.

## Discussion TODO

### Proposal N - New [active]
- Next Question: New question?

"""

        root = self._write_discussions_file(f"# Discussions\n\n{existing}")
        before, _manifest = runner._load_discussion_documents(str(root))
        before_id = [
            record["id"]
            for record in runner._build_discussion_records(before)
            if "Proposal E - Existing" in record["document"]
        ]

        (root / ".gwt" / "work" / "discussions.md").write_text(
            f"# Discussions\n\n{prefix}{existing}", encoding="utf-8"
        )
        after, _manifest = runner._load_discussion_documents(str(root))
        after_id = [
            record["id"]
            for record in runner._build_discussion_records(after)
            if "Proposal E - Existing" in record["document"]
        ]

        self.assertEqual(len(before_id), 1, before_id)
        self.assertEqual(len(after_id), 1, after_id)
        self.assertEqual(before_id, after_id)

    def test_standalone_legacy_h2_entries_keep_single_chunk_ids(self):
        legacy_entries = """# Discussions

## Discussion TODO

Legacy-only TODO before any dated discussion.

## Legacy gwt-discussion state

Legacy state before a dated entry.

## 2026-07-26 — Canonical discussion

Status: completed

Summary:
Canonical body.

## Standalone legacy tail

Legacy state after a dated entry.
"""
        root = self._write_discussions_file(legacy_entries)

        discussions, _manifest = runner._load_discussion_documents(str(root))
        records = runner._build_discussion_records(discussions)

        expected = {
            "Discussion TODO": "## Discussion TODO\n## Discussion TODO\n\nLegacy-only TODO before any dated discussion.",
            "Legacy gwt-discussion state": "## Legacy gwt-discussion state\n## Legacy gwt-discussion state\n\nLegacy state before a dated entry.",
            "Standalone legacy tail": "## Standalone legacy tail\n## Standalone legacy tail\n\nLegacy state after a dated entry.",
        }
        for title, digest_source in expected.items():
            matches = [
                record for record in records if record["metadata"]["title"] == title
            ]
            self.assertEqual(len(matches), 1, (title, records))
            expected_digest = hashlib.sha1(digest_source.encode("utf-8")).hexdigest()[:12]
            self.assertEqual(matches[0]["id"], f"discussion-{expected_digest}")
            self.assertEqual(matches[0]["metadata"]["chunk_idx"], 0)
            self.assertEqual(matches[0]["metadata"]["total_chunks"], 1)

    def test_nested_todo_keeps_dated_parent_across_another_bare_h2(self):
        nested_entries = """# Discussions

## 2026-07-26 — Canonical discussion

Status: active

Summary:
Canonical body.

## Evidence

Intermediate nested evidence.

## Discussion TODO

### Proposal A - Continue work [active]
- Next Question: Continue?
"""
        root = self._write_discussions_file(nested_entries)

        discussions, _manifest = runner._load_discussion_documents(str(root))
        records = runner._build_discussion_records(discussions)
        canonical = next(
            record
            for record in records
            if record["metadata"]["title"] == "Canonical discussion"
        )
        evidence = next(
            record
            for record in records
            if record["metadata"]["title"] == "Evidence"
        )
        todo = next(
            record
            for record in records
            if record["metadata"]["title"] == "Discussion TODO"
        )

        self.assertEqual(canonical["metadata"]["chunk_idx"], 0)
        self.assertEqual(canonical["metadata"]["total_chunks"], 2)
        self.assertEqual(todo["metadata"]["chunk_idx"], 1)
        self.assertEqual(todo["metadata"]["total_chunks"], 2)
        self.assertEqual(evidence["metadata"]["chunk_idx"], 0)
        self.assertEqual(evidence["metadata"]["total_chunks"], 1)

    def test_migrated_legacy_wrapper_breaks_dated_parent_before_todo(self):
        migrated_entries = """# Discussions

## 2026-07-26 — Canonical discussion

Status: completed

Summary:
Canonical body.

## Legacy gwt-discussion state

Status: active

Summary:
Migrated from legacy .gwt/discussion.md.

## Discussion TODO

### Proposal Legacy - Resume [active]
- Next Question: Resume legacy state?
"""
        root = self._write_discussions_file(migrated_entries)

        discussions, _manifest = runner._load_discussion_documents(str(root))
        records = runner._build_discussion_records(discussions)
        by_title = {record["metadata"]["title"]: record for record in records}

        self.assertEqual(by_title["Canonical discussion"]["metadata"]["total_chunks"], 1)
        self.assertEqual(by_title["Legacy gwt-discussion state"]["metadata"]["total_chunks"], 1)
        self.assertEqual(by_title["Discussion TODO"]["metadata"]["chunk_idx"], 0)
        self.assertEqual(by_title["Discussion TODO"]["metadata"]["total_chunks"], 1)
        todo_digest = hashlib.sha1(
            (
                "## Discussion TODO\n## Discussion TODO\n\n"
                "### Proposal Legacy - Resume [active]\n"
                "- Next Question: Resume legacy state?"
            ).encode("utf-8")
        ).hexdigest()[:12]
        self.assertEqual(
            by_title["Discussion TODO"]["id"], f"discussion-{todo_digest}"
        )

    def test_adding_same_title_different_body_keeps_legacy_record_id(self):
        existing = """## Legacy state

Existing legacy body.
"""
        prefix = """## Legacy state

Different earlier legacy body.

"""
        root = self._write_discussions_file(f"# Discussions\n\n{existing}")
        before, _manifest = runner._load_discussion_documents(str(root))
        before_id = [
            record["id"]
            for record in runner._build_discussion_records(before)
            if "Existing legacy body." in record["document"]
        ]

        (root / ".gwt" / "work" / "discussions.md").write_text(
            f"# Discussions\n\n{prefix}{existing}", encoding="utf-8"
        )
        after, _manifest = runner._load_discussion_documents(str(root))
        after_id = [
            record["id"]
            for record in runner._build_discussion_records(after)
            if "Existing legacy body." in record["document"]
        ]

        self.assertEqual(len(before_id), 1, before_id)
        self.assertEqual(len(after_id), 1, after_id)
        self.assertEqual(before_id, after_id)

    def test_identical_standalone_legacy_entries_receive_unique_record_ids(self):
        duplicate = """# Discussions

## Legacy state

Identical legacy body.

## Legacy state

Identical legacy body.
"""
        root = self._write_discussions_file(duplicate)

        discussions, _manifest = runner._load_discussion_documents(str(root))
        records = runner._build_discussion_records(discussions)
        ids = [record["id"] for record in records]

        self.assertEqual(len(ids), 2)
        self.assertEqual(len(ids), len(set(ids)), ids)
        old_digest = hashlib.sha1(
            "## Legacy state\n## Legacy state\n\nIdentical legacy body.".encode("utf-8")
        ).hexdigest()[:12]
        self.assertEqual(ids[0], f"discussion-{old_digest}")


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
        with tempfile.TemporaryDirectory() as wt, tempfile.TemporaryDirectory() as db_root_dir, tempfile.TemporaryDirectory() as home:
            root = Path(wt)
            work_dir = root / ".gwt" / "work"
            work_dir.mkdir(parents=True, exist_ok=True)
            (work_dir / "discussions.md").write_text(SAMPLE_DISCUSSIONS, encoding="utf-8")
            collection = _FakeCollection()

            previous_home = os.environ.get("HOME")
            os.environ["HOME"] = home
            try:
                with mock.patch.object(
                    runner,
                    "_make_chroma_collection_repairing",
                    return_value=(_FakeClient(), collection),
                ), mock.patch.object(runner, "_close_chroma_client"), mock.patch.object(
                    runner, "_finish_full_build", return_value=None
                ):
                    result = runner.action_index_discussions_v2(
                        project_root=str(root),
                        repo_hash="abc1234567890def",
                        worktree_hash=None,
                        mode="full",
                        db_root=Path(db_root_dir),
                    )
            finally:
                if previous_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = previous_home

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
            self.assertEqual(entries[0]["path"], str(work_dir / "discussions.md"))

    def test_full_mode_upserts_repeated_nested_h2_with_unique_ids(self):
        with (
            tempfile.TemporaryDirectory() as wt,
            tempfile.TemporaryDirectory() as db_root_dir,
            tempfile.TemporaryDirectory() as home,
        ):
            root = Path(wt)
            work_dir = root / ".gwt" / "work"
            work_dir.mkdir(parents=True, exist_ok=True)
            (work_dir / "discussions.md").write_text(
                REPEATED_DISCUSSION_TODO, encoding="utf-8"
            )
            collection = _FakeCollection()

            previous_home = os.environ.get("HOME")
            os.environ["HOME"] = home
            try:
                with mock.patch.object(
                    runner,
                    "_make_chroma_collection_repairing",
                    return_value=(_FakeClient(), collection),
                ), mock.patch.object(runner, "_close_chroma_client"), mock.patch.object(
                    runner, "_finish_full_build", return_value=None
                ):
                    result = runner.action_index_discussions_v2(
                        project_root=str(root),
                        repo_hash="abc1234567890def",
                        worktree_hash=None,
                        mode="full",
                        db_root=Path(db_root_dir),
                    )
            finally:
                if previous_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = previous_home

            self.assertTrue(result.get("ok"), result)
            self.assertEqual(len(collection.upserts), 1)
            ids = collection.upserts[0]["ids"]
            self.assertEqual(len(ids), len(set(ids)), ids)


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
        if len(ids) != len(set(ids)) or any(record_id in self.ids for record_id in ids):
            raise ValueError(f"duplicate ids across upserts: {ids}")
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
