"""Tests for the lessons (post-mortem) scope indexer.

Lessons live in `tasks/lessons.md` as H2 sections with the canonical shape:

    ## YYYY-MM-DD — title
    ### 事象
    ...
    ### 原因
    ...
    ### 再発防止策
    ...

A small minority of sections at the tail of the file lack the date prefix.
The chunker must handle both shapes without raising.
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import chroma_index_runner as runner


SAMPLE_LESSONS = """# Lessons Learned

## 2026-05-20 — alpha section title

### 事象
alpha symptom.

### 原因
alpha cause.

### 再発防止策
1. alpha prevention.

## 2026-05-19 — beta section title

### 事象
beta symptom.

### 原因
beta cause.

## undated tail section about manual reflection

This section omits the date prefix and uses a single paragraph format.
"""


class LoadLessonsDocumentsTests(unittest.TestCase):
    def _write_lessons_file(self, contents: str) -> Path:
        tmp = tempfile.mkdtemp()
        root = Path(tmp)
        tasks = root / "tasks"
        tasks.mkdir(parents=True, exist_ok=True)
        (tasks / "lessons.md").write_text(contents, encoding="utf-8")
        return root

    def test_returns_chunks_for_each_h2_section(self):
        root = self._write_lessons_file(SAMPLE_LESSONS)
        lessons, manifest = runner._load_lessons_documents(str(root))
        # 3 H2 sections in SAMPLE_LESSONS
        self.assertEqual(len(lessons), 3)
        # manifest has exactly one entry (the single source file)
        self.assertEqual(len(manifest), 1)
        self.assertEqual(manifest[0]["path"], "tasks/lessons.md")

    def test_extracts_date_from_dated_sections(self):
        root = self._write_lessons_file(SAMPLE_LESSONS)
        lessons, _manifest = runner._load_lessons_documents(str(root))
        dates = [lesson["date"] for lesson in lessons]
        self.assertIn("2026-05-20", dates)
        self.assertIn("2026-05-19", dates)

    def test_date_less_section_uses_empty_string(self):
        root = self._write_lessons_file(SAMPLE_LESSONS)
        lessons, _manifest = runner._load_lessons_documents(str(root))
        undated = [lesson for lesson in lessons if lesson["date"] == ""]
        self.assertEqual(len(undated), 1)
        self.assertIn("undated tail section", undated[0]["title"])

    def test_lesson_id_is_stable_for_same_content(self):
        root = self._write_lessons_file(SAMPLE_LESSONS)
        first, _ = runner._load_lessons_documents(str(root))
        second, _ = runner._load_lessons_documents(str(root))
        self.assertEqual([l["lesson_id"] for l in first], [l["lesson_id"] for l in second])

    def test_missing_lessons_file_returns_empty(self):
        tmp = Path(tempfile.mkdtemp())
        lessons, manifest = runner._load_lessons_documents(str(tmp))
        self.assertEqual(lessons, [])
        self.assertEqual(manifest, [])

    def test_empty_lessons_file_returns_empty(self):
        root = self._write_lessons_file("")
        lessons, manifest = runner._load_lessons_documents(str(root))
        self.assertEqual(lessons, [])
        # manifest still records the file presence so unhealthy detection can work
        # (single-file scope behaviour)
        self.assertEqual(len(manifest), 1)


class BuildLessonRecordsTests(unittest.TestCase):
    def test_returns_chroma_records_with_metadata(self):
        lessons = [
            {
                "lesson_id": "abc123def456",
                "date": "2026-05-20",
                "title": "alpha",
                "heading": "## 2026-05-20 — alpha",
                "body": "### 事象\nalpha symptom.",
                "chunk_idx": 0,
                "total_chunks": 1,
            }
        ]
        records = runner._build_lesson_records(lessons)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["id"], "lesson-abc123def456")
        self.assertIn("alpha", records[0]["document"])
        meta = records[0]["metadata"]
        self.assertEqual(meta["date"], "2026-05-20")
        self.assertEqual(meta["title"], "alpha")
        self.assertEqual(meta["chunk_idx"], 0)


class ActionIndexLessonsTests(unittest.TestCase):
    def test_full_mode_writes_manifest_and_chunks(self):
        with tempfile.TemporaryDirectory() as wt, tempfile.TemporaryDirectory() as db_root_dir:
            root = Path(wt)
            tasks = root / "tasks"
            tasks.mkdir(parents=True, exist_ok=True)
            (tasks / "lessons.md").write_text(SAMPLE_LESSONS, encoding="utf-8")

            result = runner.action_index_lessons_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash=None,
                mode="full",
                db_root=Path(db_root_dir),
            )
            self.assertTrue(result.get("ok"), result)
            self.assertEqual(result["scope"], "lessons")
            self.assertGreaterEqual(result["indexed"], 3)

            db_path = runner.resolve_db_path(
                "abc1234567890def", None, "lessons", db_root=Path(db_root_dir)
            )
            manifest_file = runner._manifest_path(db_path, "lessons")
            self.assertTrue(manifest_file.is_file(), f"missing manifest at {manifest_file}")
            manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
            entries = manifest.get("entries") if isinstance(manifest, dict) else manifest
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["path"], "tasks/lessons.md")

    def test_missing_lessons_file_returns_indexed_zero(self):
        with tempfile.TemporaryDirectory() as wt, tempfile.TemporaryDirectory() as db_root_dir:
            result = runner.action_index_lessons_v2(
                project_root=wt,
                repo_hash="abc1234567890def",
                worktree_hash=None,
                mode="full",
                db_root=Path(db_root_dir),
            )
            self.assertTrue(result.get("ok"), result)
            self.assertEqual(result["indexed"], 0)


class FormatLessonsResultsTests(unittest.TestCase):
    def test_collapses_chunks_by_date_title_and_limits_to_n(self):
        items = [
            {
                "id": "lesson-aaa",
                "distance": 0.12,
                "metadata": {
                    "lesson_id": "aaa",
                    "date": "2026-05-20",
                    "title": "alpha",
                    "heading": "## 2026-05-20 — alpha",
                    "chunk_idx": 0,
                    "total_chunks": 2,
                },
            },
            {
                "id": "lesson-aaa-2",
                "distance": 0.15,
                "metadata": {
                    "lesson_id": "aaa-2",
                    "date": "2026-05-20",
                    "title": "alpha",
                    "heading": "## 2026-05-20 — alpha [2]",
                    "chunk_idx": 1,
                    "total_chunks": 2,
                },
            },
            {
                "id": "lesson-bbb",
                "distance": 0.20,
                "metadata": {
                    "lesson_id": "bbb",
                    "date": "2026-05-19",
                    "title": "beta",
                    "heading": "## 2026-05-19 — beta",
                    "chunk_idx": 0,
                    "total_chunks": 1,
                },
            },
        ]
        out = runner._format_lessons_results(items, n_results=10)
        # 2 unique (date, title) groups
        self.assertEqual(len(out), 2)
        # best-scoring chunk wins for each group
        first = out[0]
        self.assertEqual(first["date"], "2026-05-20")
        self.assertEqual(first["title"], "alpha")


if __name__ == "__main__":
    unittest.main()
