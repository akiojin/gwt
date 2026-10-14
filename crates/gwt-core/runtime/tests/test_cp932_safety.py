"""Regression tests for cp932 decode failures on Japanese Windows.

The runtime previously used Path.read_text() / Path.write_text() without
specifying an encoding. On Japanese Windows, locale.getpreferredencoding
returns "cp932", so UTF-8 content written by gwt (issue cache meta.json,
body.md, spec.md etc.) fails to decode when read back.

These tests pin the expected behavior: production code must force UTF-8 so
that file I/O is independent of the system locale.
"""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


_REAL_OPEN = io.open


def _cp932_default_open(
    file,
    mode="r",
    buffering=-1,
    encoding=None,
    errors=None,
    newline=None,
    *args,
    **kwargs,
):
    """Simulate Windows Japanese locale: when the caller does not pass an
    encoding and opens the file in text mode, default to cp932 instead of
    the host's actual locale encoding (usually UTF-8 on CI).

    pathlib's Path.open forwards positional arguments to io.open as
    (file, mode, buffering, encoding, errors, newline), so we must accept
    them positionally to catch the bug before bytes reach the decoder."""
    if "b" not in mode and encoding is None:
        encoding = "cp932"
    return _REAL_OPEN(
        file, mode, buffering, encoding, errors, newline, *args, **kwargs
    )


class Cp932SafetyTests(unittest.TestCase):
    def _write_utf8_cache(self, cache_root: Path, number: int) -> None:
        issue = cache_root / str(number)
        issue.mkdir(parents=True, exist_ok=True)
        meta = {
            "number": number,
            "title": "秦泉寺の章夫テスト",
            "labels": ["bug"],
            "state": "open",
            "updated_at": "2026-04-23T00:00:00Z",
            "comment_ids": [],
        }
        # Explicit UTF-8 write so we test the read path, not the write path.
        (issue / "meta.json").write_text(
            json.dumps(meta, ensure_ascii=False), encoding="utf-8"
        )
        (issue / "body.md").write_text(
            "本文には日本語文字列 (秦泉寺章夫) が含まれる。\n"
            "position 172 以降に 0x94 を踏むような文字列テスト。\n"
            "端末エンコーディング依存のバグを防ぐ。",
            encoding="utf-8",
        )

    def test_load_cached_issue_documents_under_cp932_locale(self):
        """_load_cached_issue_documents must decode UTF-8 cache even when the
        default locale is cp932 (Japanese Windows)."""
        with tempfile.TemporaryDirectory() as tmp:
            repo_hash = "abc1234567890def"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / repo_hash
            self._write_utf8_cache(cache_root, 42)

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                with mock.patch("io.open", side_effect=_cp932_default_open):
                    issues = runner._load_cached_issue_documents(repo_hash)

            self.assertEqual(len(issues), 1, issues)
            self.assertEqual(issues[0]["number"], 42)
            self.assertEqual(issues[0]["title"], "秦泉寺の章夫テスト")
            self.assertIn("秦泉寺章夫", issues[0]["body"])

    def test_read_issue_meta_under_cp932_locale(self):
        """_read_issue_meta reads the db-level meta.json which records
        last_full_refresh and TTL. If this file is written in UTF-8 (the
        common case when gwt runs on macOS/Linux and the cache is then
        synced to Windows, or after this fix), reading it under a cp932
        default locale must still succeed."""
        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "issues"
            db_path.mkdir(parents=True)
            payload = {
                "schema_version": 1,
                "last_full_refresh": "2026-04-23T00:00:00+00:00",
                "ttl_minutes": 15,
                "note": "日本語コメントを含む",
            }
            (db_path / runner.META_FILENAME).write_text(
                json.dumps(payload, ensure_ascii=False), encoding="utf-8"
            )

            with mock.patch("io.open", side_effect=_cp932_default_open):
                meta = runner._read_issue_meta(db_path)

            self.assertIsNotNone(meta)
            self.assertEqual(meta["last_full_refresh"], payload["last_full_refresh"])
            self.assertEqual(meta["note"], "日本語コメントを含む")

    def test_load_cached_spec_documents_under_cp932_locale(self):
        """_load_cached_spec_documents reads gwt-spec labelled issues with
        their sections/spec.md content. Japanese spec text must round-trip
        cleanly under a cp932 default locale."""
        with tempfile.TemporaryDirectory() as tmp:
            repo_hash = "abc1234567890def"
            cache_root = Path(tmp) / ".gwt" / "cache" / "issues" / repo_hash
            issue = cache_root / "1930"
            (issue / "sections").mkdir(parents=True, exist_ok=True)
            (issue / "meta.json").write_text(
                json.dumps(
                    {
                        "number": 1930,
                        "title": "SPEC-1930: 秦泉寺仕様",
                        "labels": ["gwt-spec"],
                        "state": "open",
                    },
                    ensure_ascii=False,
                ),
                encoding="utf-8",
            )
            (issue / "sections" / "spec.md").write_text(
                "# 仕様\n\n秦泉寺章夫がオーナーの SPEC。\n",
                encoding="utf-8",
            )

            with mock.patch.dict(os.environ, {"HOME": tmp}, clear=False):
                with mock.patch("io.open", side_effect=_cp932_default_open):
                    specs, _ = runner._load_cached_spec_documents(repo_hash)

            self.assertEqual(len(specs), 1, specs)
            self.assertEqual(specs[0]["spec_id"], "1930")
            self.assertIn("秦泉寺章夫", specs[0]["content"])
            self.assertEqual(specs[0]["title"], "SPEC-1930: 秦泉寺仕様")


class NonAsciiStorePathTests(unittest.TestCase):
    """Issue #4205: chromadb's HNSW layer cannot persist its `.bin` files
    under a non-ASCII directory on Windows (a Japanese user-profile name puts
    every index store there). The in-process build still counts correctly, so
    the rebuild reports success, but every fresh process fails to load the
    index, reads it as 0 documents and schedules the same rebuild forever."""

    # Above chromadb's default HNSW sync threshold (1000), so the build
    # flushes the index to disk instead of replaying it from the SQLite log.
    VECTORS = 1100

    def test_hnsw_store_reopens_in_fresh_process_under_non_ascii_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            # resolve() restores the long name of a short-form %TEMP%, which is
            # the production shape: a non-ASCII profile directory with an 8.3
            # alias. Elsewhere, add a non-ASCII component of our own.
            base = Path(tmp).resolve()
            if str(base).isascii():
                base = base / "秦泉寺章夫"
            store = base / "issues"
            store.mkdir(parents=True)
            try:
                client, collection = runner._make_file_index_v2_collection(store, "probe")
            except runner.IndexStorePathError:
                self.skipTest("volume has no 8.3 alias for the non-ASCII directory")
            try:
                for start in range(0, self.VECTORS, 100):
                    ids = [str(i) for i in range(start, min(self.VECTORS, start + 100))]
                    collection.upsert(
                        ids=ids,
                        embeddings=[[((int(i) * 7 + j) % 13 + 1) / 13.0 for j in range(8)] for i in ids],
                    )
            finally:
                runner._close_chroma_client(client)

            reopen = (
                "import sys; sys.path.insert(0, sys.argv[1]);"
                "from pathlib import Path; import chroma_index_runner as runner;"
                "client, collection = runner._open_file_index_v2_collection(Path(sys.argv[2]), 'probe');"
                "print(collection.count()); runner._close_chroma_client(client)"
            )
            result = subprocess.run(
                [sys.executable, "-c", reopen, str(Path(runner.__file__).parent), str(store)],
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=300,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), str(self.VECTORS), result.stderr)

    @unittest.skipUnless(os.name == "nt", "Windows short-path fallback")
    def test_store_path_without_ascii_alias_fails_loudly(self):
        with tempfile.TemporaryDirectory() as tmp:
            store = Path(tmp) / "秦泉寺章夫"
            store.mkdir()
            with mock.patch.object(runner, "_windows_short_path", side_effect=lambda text: text):
                with self.assertRaises(runner.IndexStorePathError):
                    runner._chroma_store_path(store)


if __name__ == "__main__":
    unittest.main()
