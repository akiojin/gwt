"""Phase 8: tests for portalocker-based DB directory locking.

Writers must serialize via LOCK_EX, readers via LOCK_SH. Locks must release
even when the operation raises.
"""

from __future__ import annotations

import multiprocessing
import os
import tempfile
import time
import unittest
from pathlib import Path

import chroma_index_runner as runner


def _writer_holding_lock(db_path: str, hold_seconds: float, marker_path: str) -> None:
    """Subprocess entrypoint: hold the exclusive lock for hold_seconds."""
    with runner.acquire_lock(Path(db_path), exclusive=True):
        Path(marker_path).write_text(str(time.time()))
        time.sleep(hold_seconds)


def _writer_raises(db_path: str, marker_path: str) -> None:
    try:
        with runner.acquire_lock(Path(db_path), exclusive=True):
            Path(marker_path).write_text("entered")
            raise RuntimeError("boom")
    except RuntimeError:
        pass


def _writer_records_acquired_at(db_path: str, queue: multiprocessing.Queue) -> None:
    with runner.acquire_lock(Path(db_path), exclusive=True):
        queue.put(time.time())
        time.sleep(0.3)


class FlockTests(unittest.TestCase):
    def test_acquire_lock_helper_exists(self):
        self.assertTrue(
            hasattr(runner, "acquire_lock"),
            "Phase 8 must add acquire_lock context manager to the runner",
        )

    def test_two_writers_serialize(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp) / "db"
            db.mkdir()

            ctx = multiprocessing.get_context("spawn")
            queue = ctx.Queue()

            p1 = ctx.Process(target=_writer_records_acquired_at, args=(str(db), queue))
            p2 = ctx.Process(target=_writer_records_acquired_at, args=(str(db), queue))
            p1.start()
            time.sleep(0.05)
            p2.start()
            p1.join(timeout=5)
            p2.join(timeout=5)

            self.assertFalse(p1.is_alive())
            self.assertFalse(p2.is_alive())
            self.assertEqual(p1.exitcode, 0)
            self.assertEqual(p2.exitcode, 0)

            t1 = queue.get(timeout=1)
            t2 = queue.get(timeout=1)
            self.assertGreaterEqual(
                abs(t2 - t1),
                0.25,
                "second writer must wait for first writer to release",
            )

    def test_lock_released_on_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp) / "db"
            db.mkdir()
            marker = Path(tmp) / "marker"

            ctx = multiprocessing.get_context("spawn")
            p1 = ctx.Process(target=_writer_raises, args=(str(db), str(marker)))
            p1.start()
            p1.join(timeout=5)

            # If the lock leaked, this acquisition would hang.
            with runner.acquire_lock(db, exclusive=True):
                self.assertEqual(marker.read_text(), "entered")

    def test_reader_acquires_after_writer_releases(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp) / "db"
            db.mkdir()
            marker = Path(tmp) / "writer-marker"

            ctx = multiprocessing.get_context("spawn")
            p1 = ctx.Process(
                target=_writer_holding_lock, args=(str(db), 0.5, str(marker))
            )
            p1.start()

            # Wait until writer is in the critical section.
            for _ in range(50):
                if marker.exists():
                    break
                time.sleep(0.02)
            self.assertTrue(marker.exists())

            t_before = time.time()
            with runner.acquire_lock(db, exclusive=False):
                t_acquired = time.time()
            p1.join(timeout=5)

            self.assertGreaterEqual(
                t_acquired - t_before,
                0.2,
                "reader must wait for writer to release",
            )

    def test_lock_sentinel_file_lives_in_db_dir(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp) / "db"
            db.mkdir()
            with runner.acquire_lock(db, exclusive=True):
                self.assertTrue((db / ".lock").exists())


if __name__ == "__main__":
    unittest.main()
