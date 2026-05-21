"""Tests for the memory (post-mortem) scope indexer.

Memory entries live in `tasks/memory.md` as H2 sections with the canonical shape:

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


SAMPLE_MEMORY = """# Memory Learned

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


class LoadMemoryDocumentsTests(unittest.TestCase):
    def _write_memory_file(self, contents: str) -> Path:
        tmp = tempfile.mkdtemp()
        root = Path(tmp)
        tasks = root / "tasks"
        tasks.mkdir(parents=True, exist_ok=True)
        (tasks / "memory.md").write_text(contents, encoding="utf-8")
        return root

    def test_returns_chunks_for_each_h2_section(self):
        root = self._write_memory_file(SAMPLE_MEMORY)
        memory, manifest = runner._load_memory_documents(str(root))
        # 3 H2 sections in SAMPLE_MEMORY
        self.assertEqual(len(memory), 3)
        # manifest has exactly one entry (the single source file)
        self.assertEqual(len(manifest), 1)
        self.assertEqual(manifest[0]["path"], "tasks/memory.md")

    def test_extracts_date_from_dated_sections(self):
        root = self._write_memory_file(SAMPLE_MEMORY)
        memory, _manifest = runner._load_memory_documents(str(root))
        dates = [memory["date"] for memory in memory]
        self.assertIn("2026-05-20", dates)
        self.assertIn("2026-05-19", dates)

    def test_date_less_section_uses_empty_string(self):
        root = self._write_memory_file(SAMPLE_MEMORY)
        memory, _manifest = runner._load_memory_documents(str(root))
        undated = [memory for memory in memory if memory["date"] == ""]
        self.assertEqual(len(undated), 1)
        self.assertIn("undated tail section", undated[0]["title"])

    def test_memory_id_is_stable_for_same_content(self):
        root = self._write_memory_file(SAMPLE_MEMORY)
        first, _ = runner._load_memory_documents(str(root))
        second, _ = runner._load_memory_documents(str(root))
        self.assertEqual([l["memory_id"] for l in first], [l["memory_id"] for l in second])

    def test_missing_memory_file_returns_empty(self):
        tmp = Path(tempfile.mkdtemp())
        memory, manifest = runner._load_memory_documents(str(tmp))
        self.assertEqual(memory, [])
        self.assertEqual(manifest, [])

    def test_empty_memory_file_returns_empty(self):
        root = self._write_memory_file("")
        memory, manifest = runner._load_memory_documents(str(root))
        self.assertEqual(memory, [])
        # manifest still records the file presence so unhealthy detection can work
        # (single-file scope behaviour)
        self.assertEqual(len(manifest), 1)


class BuildMemoryRecordsTests(unittest.TestCase):
    def test_returns_chroma_records_with_metadata(self):
        memory = [
            {
                "memory_id": "abc123def456",
                "date": "2026-05-20",
                "title": "alpha",
                "heading": "## 2026-05-20 — alpha",
                "body": "### 事象\nalpha symptom.",
                "chunk_idx": 0,
                "total_chunks": 1,
            }
        ]
        records = runner._build_memory_records(memory)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["id"], "memory-abc123def456")
        self.assertIn("alpha", records[0]["document"])
        meta = records[0]["metadata"]
        self.assertEqual(meta["date"], "2026-05-20")
        self.assertEqual(meta["title"], "alpha")
        self.assertEqual(meta["chunk_idx"], 0)


class ActionIndexMemoryTests(unittest.TestCase):
    def test_full_mode_writes_manifest_and_chunks(self):
        with tempfile.TemporaryDirectory() as wt, tempfile.TemporaryDirectory() as db_root_dir:
            root = Path(wt)
            tasks = root / "tasks"
            tasks.mkdir(parents=True, exist_ok=True)
            (tasks / "memory.md").write_text(SAMPLE_MEMORY, encoding="utf-8")

            result = runner.action_index_memory_v2(
                project_root=str(root),
                repo_hash="abc1234567890def",
                worktree_hash=None,
                mode="full",
                db_root=Path(db_root_dir),
            )
            self.assertTrue(result.get("ok"), result)
            self.assertEqual(result["scope"], "memory")
            self.assertGreaterEqual(result["indexed"], 3)

            db_path = runner.resolve_db_path(
                "abc1234567890def", None, "memory", db_root=Path(db_root_dir)
            )
            manifest_file = runner._manifest_path(db_path, "memory")
            self.assertTrue(manifest_file.is_file(), f"missing manifest at {manifest_file}")
            manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
            entries = manifest.get("entries") if isinstance(manifest, dict) else manifest
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["path"], "tasks/memory.md")

    def test_missing_memory_file_returns_indexed_zero(self):
        with tempfile.TemporaryDirectory() as wt, tempfile.TemporaryDirectory() as db_root_dir:
            result = runner.action_index_memory_v2(
                project_root=wt,
                repo_hash="abc1234567890def",
                worktree_hash=None,
                mode="full",
                db_root=Path(db_root_dir),
            )
            self.assertTrue(result.get("ok"), result)
            self.assertEqual(result["indexed"], 0)


class FormatMemoryResultsTests(unittest.TestCase):
    def test_collapses_chunks_by_date_title_and_limits_to_n(self):
        items = [
            {
                "id": "memory-aaa",
                "distance": 0.12,
                "metadata": {
                    "memory_id": "aaa",
                    "date": "2026-05-20",
                    "title": "alpha",
                    "heading": "## 2026-05-20 — alpha",
                    "chunk_idx": 0,
                    "total_chunks": 2,
                },
            },
            {
                "id": "memory-aaa-2",
                "distance": 0.15,
                "metadata": {
                    "memory_id": "aaa-2",
                    "date": "2026-05-20",
                    "title": "alpha",
                    "heading": "## 2026-05-20 — alpha [2]",
                    "chunk_idx": 1,
                    "total_chunks": 2,
                },
            },
            {
                "id": "memory-bbb",
                "distance": 0.20,
                "metadata": {
                    "memory_id": "bbb",
                    "date": "2026-05-19",
                    "title": "beta",
                    "heading": "## 2026-05-19 — beta",
                    "chunk_idx": 0,
                    "total_chunks": 1,
                },
            },
        ]
        out = runner._format_memory_results(items, n_results=10)
        # 2 unique (date, title) groups
        self.assertEqual(len(out), 2)
        # best-scoring chunk wins for each group
        first = out[0]
        self.assertEqual(first["date"], "2026-05-20")
        self.assertEqual(first["title"], "alpha")


if __name__ == "__main__":
    unittest.main()
