"""SPEC #5429 AC-4: project-scoped PM report search and document differences."""

import json
import os
import sys
from unittest import mock

import pytest

import chroma_index_runner as runner


@pytest.fixture
def corpus(tmp_path, monkeypatch):
    home = tmp_path / "home"
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))
    state = home / ".gwt/projects/reports-repo/project-state"
    state.mkdir(parents=True)
    ledger = state / "pm-reports.jsonl"
    report = {
        "id": "digest-one", "kind": "progress",
        "body": "Delivered watcher repair #5429 and https://github.com/example/repo/issues/5430 via PR #5487 and pUlL rEqUeSt #5488.",
        "created_at": "2026-10-15T01:02:03Z",
    }
    ledger.write_text(json.dumps(report) + "\n", encoding="utf-8")
    instruction = {
        "id": "qa-one", "created_at": "2026-10-15T01:03:04Z",
        "original": "Original user request about watcher behavior.",
        "summary": "Why did the watcher change?", "answer": "The watcher now preserves pending changes.",
        "actions": ["Preserve queued watcher updates"], "issue_numbers": [5429],
        "kind": "question", "superseded_by": None,
    }
    instructions = state / "user-instructions.json"
    instructions.write_text(json.dumps([instruction]), encoding="utf-8")
    return tmp_path / "index", ledger, instructions, report, instruction


def index(corpus, mode="full", repo_hash="reports-repo"):
    assert "reports" in runner.V2_SCOPES
    return runner.action_index_reports_v2(
        repo_hash=repo_hash, project_root=None, mode=mode, db_root=corpus[0],
    )


def search(corpus, repo_hash="reports-repo"):
    return runner.action_search_v2(
        "search-reports", repo_hash, None, None, "watcher", no_auto_build=True,
        db_root=corpus[0],
    )


def test_reports_roundtrip_preserves_metadata_and_project_isolation(corpus):
    db_root, ledger, _, report, instruction = corpus
    other = ledger.parents[2] / "other-repo/project-state/pm-reports.jsonl"
    other.parent.mkdir(parents=True)
    other.write_text(json.dumps(dict(report, id="other", body="Private other watcher report")) + "\n")
    assert index(corpus)["indexed"] == 2
    assert index(corpus, repo_hash="other-repo")["indexed"] == 1
    result = search(corpus)
    assert result["ok"], result
    hits = {hit["report_id"]: hit for hit in result["reportResults"]}
    assert set(hits) == {report["id"], instruction["id"]}
    digest, qa = hits[report["id"]], hits[instruction["id"]]
    assert digest["source"] == "pm_report"
    assert digest["kind"] == "progress" and digest["report_type"] == "digest"
    assert digest["created_at"] == report["created_at"]
    assert digest["body"] == report["body"] and digest["question"] is None
    assert digest["issue_numbers"] == [5429, 5430]
    assert qa["source"] == "user_instruction"
    assert qa["kind"] == qa["report_type"] == "qa"
    assert qa["created_at"] == instruction["created_at"]
    assert qa["question"] == instruction["summary"] and qa["body"] == instruction["answer"]
    assert qa["issue_numbers"] == instruction["issue_numbers"]
    assert {hit["report_id"] for hit in search(corpus, "other-repo")["reportResults"]} == {"other"}
    db_path = runner.resolve_db_path("reports-repo", None, "reports", db_root)
    client, collection = runner._open_chroma_collection(runner.resolve_active_store(db_path), "reports")
    try:
        document = collection.get(ids=["report-user_instruction-qa-one"], include=["documents"])["documents"][0]
        assert instruction["original"] in document and instruction["actions"][0] in document
    finally:
        runner._close_chroma_client(client)
    status = runner.action_status_v2("reports-repo", None, db_root)["status"]["reports"]
    assert status["healthy"] and status["document_count"] == status["source_document_count"] == 2


def test_incremental_noop_and_new_post_only_upsert_document_difference(corpus):
    db_root, ledger, _, report, _ = corpus
    assert index(corpus)["ok"]
    db_path = runner.resolve_db_path("reports-repo", None, "reports", db_root)
    client, collection = runner._make_chroma_collection(runner.resolve_active_store(db_path), "reports")
    try:
        recording = mock.Mock(wraps=collection)
        with mock.patch.object(runner, "_make_chroma_collection", return_value=(client, recording)), mock.patch.object(runner, "_close_chroma_client"):
            assert index(corpus, "incremental")["indexed"] == 0
            recording.upsert.assert_not_called()
            with ledger.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(dict(report, id="digest-two", body="New watcher result #5431")) + "\n")
            changed = index(corpus, "incremental")
            assert changed["indexed"] == 1 and changed["document_count"] == 3
            assert recording.upsert.call_args.kwargs["ids"] == ["report-pm_report-digest-two"]
            recording.delete.assert_not_called()
    finally:
        runner._close_chroma_client(client)


def test_search_multi_detects_same_count_same_size_report_edit(corpus):
    _, ledger, _, _, _ = corpus
    assert index(corpus)["ok"]
    def multi():
        return runner.action_search_multi_v2(
            "reports-repo", None, None, "watcher", 10, ["reports"], db_root=corpus[0],
        )
    assert multi()["scopes"]["reports"]["state"] == "fresh"
    previous = ledger.stat()
    ledger.write_text(ledger.read_text().replace("Delivered", "Completed"), encoding="utf-8")
    assert ledger.stat().st_size == previous.st_size
    os.utime(ledger, ns=(previous.st_atime_ns, previous.st_mtime_ns))
    assert multi()["stale_scopes"] == ["reports"]
    assert index(corpus, "incremental")["indexed"] == 1
    refreshed = multi()
    assert refreshed["scopes"]["reports"]["state"] == "fresh"
    assert any(hit["body"].startswith("Completed") for hit in refreshed["reportResults"])


@pytest.mark.parametrize("source", ["ledger", "instructions"])
def test_corrupt_report_source_preserves_verified_store(corpus, source):
    db_root, ledger, instructions, report, _ = corpus
    assert index(corpus)["ok"]
    (instructions.parent / "works.json").write_text(json.dumps({
        "work_items": [{"id": "work-one", "title": "Watcher repair", "intent": "Preserve watcher updates"}],
    }), encoding="utf-8")
    assert runner.action_index_works_v2(None, "reports-repo", None, db_root=db_root)["ok"]
    db_path = runner.resolve_db_path("reports-repo", None, "reports", db_root)
    pointer = runner.active_pointer_path(db_path).read_bytes()
    manifest = runner._manifest_path(db_path, "reports").read_bytes()
    if source == "ledger":
        ledger.write_text(json.dumps(report), encoding="utf-8")  # Interrupted append.
    else:
        instructions.write_text("[", encoding="utf-8")
    with pytest.raises(ValueError):
        index(corpus, "incremental")
    assert runner.active_pointer_path(db_path).read_bytes() == pointer
    assert runner._manifest_path(db_path, "reports").read_bytes() == manifest
    assert len(search(corpus)["reportResults"]) == 2
    result = runner.action_search_multi_v2(
        "reports-repo", None, None, "watcher", 10, ["reports", "works"], db_root=db_root,
    )
    assert result["ok"], result
    assert result["scopes"]["reports"] == {"state": "corrupt", "reason": "source_invalid"}
    assert result["scopes"]["works"]["state"] == "fresh"
    assert [hit["work_id"] for hit in result["workResults"]] == ["work-one"]
    assert "reports" not in result["scope_results"]


def test_reports_cli_routes_build_and_search(corpus, monkeypatch):
    assert "reports" in runner.V2_SCOPES
    emitted = []
    monkeypatch.setattr(runner, "emit", emitted.append)
    for action in ("index-reports", "search-reports"):
        monkeypatch.setattr(sys, "argv", [
            "runner", "--action", action, "--repo-hash", "reports-repo",
            "--scope", "reports", "--db-root", str(corpus[0]), "--query", "watcher",
        ])
        assert runner.main() == 0
        assert emitted[-1]["ok"], emitted[-1]
    assert len(emitted[-1]["reportResults"]) == 2
