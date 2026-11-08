"""Issue #5598: share source reads without hiding canonical cache mutations."""

import json
import os
import tempfile
import unittest
import uuid
from pathlib import Path
from unittest import mock

import chroma_index_runner as runner


class SearchSourceCacheTests(unittest.TestCase):
    def test_revision_reuses_issue_and_spec_sources_and_observes_edits(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp) / ".gwt/cache/issues/repo"
            issue = root / "1"
            issue.mkdir(parents=True)
            meta = {"number": 1, "title": "alpha", "labels": ["gwt-spec"], "state": "open"}
            (issue / "meta.json").write_text(json.dumps(meta), encoding="utf-8")
            (issue / "body.md").write_text("alpha body", encoding="utf-8")
            revision = root / "source-revision"
            revision.write_text(str(uuid.uuid4()), encoding="utf-8")
            first = runner._issue_cache_source_snapshot("repo")
            issues = runner._load_cached_issue_documents("repo")
            specs = runner._load_cached_spec_documents("repo")
            read_text = Path.read_text

            def read_stamp(path, *args, **kwargs):
                if path.name not in {"source-revision", "refresh-meta.json"}:
                    raise AssertionError("unchanged source reread: " + str(path))
                return read_text(path, *args, **kwargs)

            with mock.patch.object(Path, "read_text", read_stamp):
                self.assertEqual(runner._issue_cache_source_snapshot("repo"), first)
                self.assertEqual(runner._load_cached_issue_documents("repo"), issues)
                self.assertEqual(runner._load_cached_spec_documents("repo"), specs)
            meta["title"] = "beta"
            (issue / "meta.json").write_text(json.dumps(meta), encoding="utf-8")
            revision.write_text(str(uuid.uuid4()), encoding="utf-8")
            self.assertNotEqual(runner._issue_cache_source_snapshot("repo")["fingerprint"], first["fingerprint"])
            self.assertEqual(runner._load_cached_spec_documents("repo")[0][0]["title"], "beta")

    def test_unversioned_sources_are_read_again(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp) / ".gwt/cache/issues/repo/1"
            root.mkdir(parents=True)
            (root / "meta.json").write_text('{"title":"alpha"}', encoding="utf-8")
            first = runner._issue_cache_source_snapshot("repo")
            (root / "meta.json").write_text('{"title":"bravo"}', encoding="utf-8")
            self.assertNotEqual(runner._issue_cache_source_snapshot("repo")["fingerprint"], first["fingerprint"])

    def test_revision_change_during_read_is_not_retained(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(os.environ, {"HOME": tmp}):
            root = Path(tmp) / ".gwt/cache/issues/repo"
            root.mkdir(parents=True)
            revision = root / "source-revision"
            revision.write_text(str(uuid.uuid4()), encoding="utf-8")

            def changing_source(_repo):
                revision.write_text(str(uuid.uuid4()), encoding="utf-8")
                return []

            with mock.patch.object(runner, "_load_cached_issue_documents", side_effect=changing_source) as load:
                runner._issue_cache_source_snapshot("repo")
                runner._issue_cache_source_snapshot("repo")
                self.assertEqual(load.call_count, 2)


if __name__ == "__main__":
    unittest.main()
