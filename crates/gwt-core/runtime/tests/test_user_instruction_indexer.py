"""User instruction corpus shares memory search without replacing work notes."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

import chroma_index_runner as runner


REAL_MODEL_CACHE = Path.home() / ".cache" / "huggingface"


@pytest.fixture
def instruction_corpus(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    root = tmp_path / "repo"
    root.mkdir()
    project = tmp_path / "home" / ".gwt" / "projects" / "instructions-repo"
    notes = project / "work-notes" / "memory.md"
    notes.parent.mkdir(parents=True)
    notes.write_text("# Memory\n\n## 2026-10-13 — lesson\n\nPreserve work notes.\n", encoding="utf-8")
    source = project / "project-state" / "user-instructions.json"
    source.parent.mkdir(parents=True)
    records = [
        {
            "id": "instruction-old",
            "created_at": "2026-10-13T01:02:03Z",
            "original": "作業結果は日本語で説明してください。\n\n省略せずに判断根拠も残してください。",
            "summary": "Response language preference",
            "answer": "日本語で説明します。",
            "actions": ["日本語の報告を使用する"],
            "issue_numbers": [5342],
            "kind": "standing",
            "superseded_by": "instruction-new",
        },
        {
            "id": "instruction-new",
            "created_at": "2026-10-13T01:02:03Z",
            "original": "Please provide work results in English and include the rationale.",
            "summary": "Response language preference",
            "answer": "I will report in English.",
            "actions": ["Use English reports"],
            "issue_numbers": [5342, 5341],
            "kind": "standing",
            "superseded_by": None,
        },
    ]
    source.write_text(json.dumps(records, ensure_ascii=False), encoding="utf-8")
    return root, tmp_path / "db", source, notes, records


def index_corpus(corpus, mode="full"):
    root, db_root, *_ = corpus
    result = runner.action_index_memory_v2(
        str(root), "instructions-repo", None, mode=mode, db_root=db_root
    )
    assert result["ok"], result
    return result


def search_corpus(corpus, query, n_results=10):
    root, db_root, *_ = corpus
    result = runner.action_search_v2(
        "search-memory", "instructions-repo", None, str(root), query,
        n_results=n_results, no_auto_build=True, db_root=db_root,
    )
    assert result["ok"], result
    return result["memoryResults"]


def search_multi_corpus(corpus, query, n_results=10):
    root, db_root, *_ = corpus
    result = runner.action_search_multi_v2(
        "instructions-repo", None, str(root), query, n_results,
        scopes=["memory"], db_root=db_root,
    )
    assert result["ok"], result
    return result


@pytest.mark.parametrize("keep_notes", [True, False])
def test_saved_instructions_roundtrip_with_full_original_and_supersession(instruction_corpus, keep_notes):
    _, db_root, source, notes, records = instruction_corpus
    if not keep_notes:
        notes.unlink()
    assert index_corpus(instruction_corpus)["indexed"] == 2 + keep_notes
    hits = search_corpus(instruction_corpus, "response language")
    instructions = {hit["instruction_id"]: hit for hit in hits if hit.get("source") == "user_instruction"}
    assert set(instructions) == {record["id"] for record in records}
    for record in records:
        hit = instructions[record["id"]]
        for field in ("created_at", "original", "summary", "answer", "actions", "issue_numbers", "kind", "superseded_by"):
            assert hit[field] == record[field]
    assert instructions["instruction-old"]["status"] == "superseded"
    assert instructions["instruction-new"]["status"] == "active"
    assert any(hit["title"] == "lesson" for hit in hits) == keep_notes
    db_path = runner.resolve_db_path("instructions-repo", None, "memory", db_root=db_root)
    assert {entry["path"] for entry in runner.read_manifest(db_path, "memory")} == (
        {str(source), str(notes)} if keep_notes else {str(source)}
    )


def test_incremental_detects_same_size_same_timestamp_supersession_change(instruction_corpus):
    *_, source, _notes, records = instruction_corpus
    index_corpus(instruction_corpus)
    previous = source.stat()
    records[0]["superseded_by"] = "instruction-alt"
    source.write_text(json.dumps(records, ensure_ascii=False), encoding="utf-8")
    assert source.stat().st_size == previous.st_size
    os.utime(source, ns=(previous.st_atime_ns, previous.st_mtime_ns))
    assert index_corpus(instruction_corpus, mode="incremental")["indexed"] == 3
    hits = search_corpus(instruction_corpus, "language")
    assert next(hit for hit in hits if hit.get("instruction_id") == "instruction-old")["superseded_by"] == "instruction-alt"


def test_search_multi_marks_saved_instructions_stale_until_incremental_refresh(instruction_corpus):
    _, _, source, _, records = instruction_corpus
    source.write_text(json.dumps([dict(records[0], superseded_by=None)], ensure_ascii=False), encoding="utf-8")
    index_corpus(instruction_corpus)
    initial = search_multi_corpus(instruction_corpus, "language")
    assert initial["scopes"]["memory"]["state"] == "fresh"
    assert next(hit for hit in initial["memoryResults"] if hit.get("instruction_id") == "instruction-old")["status"] == "active"

    source.write_text(json.dumps(records, ensure_ascii=False), encoding="utf-8")
    stale = search_multi_corpus(instruction_corpus, "language")
    assert stale["scopes"]["memory"]["state"] == "stale"
    assert stale["stale_scopes"] == ["memory"]
    index_corpus(instruction_corpus, mode="incremental")
    refreshed = search_multi_corpus(instruction_corpus, "language")
    assert refreshed["scopes"]["memory"]["state"] == "fresh"
    instructions = {hit["instruction_id"]: hit for hit in refreshed["memoryResults"] if hit.get("source") == "user_instruction"}
    assert "instruction-new" in instructions
    assert instructions["instruction-new"]["original"] == records[1]["original"]
    assert instructions["instruction-old"]["status"] == "superseded"

    previous = source.stat()
    records[0]["superseded_by"] = "instruction-alt"
    source.write_text(json.dumps(records, ensure_ascii=False), encoding="utf-8")
    assert source.stat().st_size == previous.st_size
    os.utime(source, ns=(previous.st_atime_ns, previous.st_mtime_ns))
    stale = search_multi_corpus(instruction_corpus, "language")
    assert stale["scopes"]["memory"]["state"] == "stale"
    assert stale["stale_scopes"] == ["memory"]
    index_corpus(instruction_corpus, mode="incremental")
    refreshed = search_multi_corpus(instruction_corpus, "language")
    assert refreshed["scopes"]["memory"]["state"] == "fresh"
    assert next(hit for hit in refreshed["memoryResults"] if hit.get("instruction_id") == "instruction-old")["superseded_by"] == "instruction-alt"


def test_deleted_instruction_corpus_removes_hits_and_preserves_work_notes(instruction_corpus):
    _, db_root, source, notes, _ = instruction_corpus
    index_corpus(instruction_corpus)
    source.unlink()
    assert index_corpus(instruction_corpus, mode="incremental")["indexed"] == 1
    hits = search_corpus(instruction_corpus, "language")
    assert [hit["title"] for hit in hits] == ["lesson"]
    db_path = runner.resolve_db_path("instructions-repo", None, "memory", db_root=db_root)
    assert [entry["path"] for entry in runner.read_manifest(db_path, "memory")] == [str(notes)]


@pytest.mark.parametrize("contents", ["[", "{}", '[{"id":"missing-fields"}]'])
def test_corrupt_instruction_corpus_fails_without_replacing_existing_index(instruction_corpus, contents):
    root, db_root, source, _, _ = instruction_corpus
    index_corpus(instruction_corpus)
    source.write_text(contents, encoding="utf-8")
    with pytest.raises(ValueError, match="user instruction corpus"):
        runner.action_index_memory_v2(
            str(root), "instructions-repo", None, mode="incremental", db_root=db_root
        )
    assert len(search_corpus(instruction_corpus, "language")) == 3
    with pytest.raises(ValueError, match="user instruction corpus"):
        search_multi_corpus(instruction_corpus, "language")


@pytest.mark.skipif(
    not os.environ.get("GWT_INSTRUCTION_GWTD"),
    reason="opt-in built gwtd instruction capture integration",
)
def test_gwtd_capture_roundtrips_through_memory_and_fresh_pm_startup(tmp_path, monkeypatch):
    binary = Path(os.environ["GWT_INSTRUCTION_GWTD"])
    assert binary.is_absolute() and binary.is_file()
    home, root = tmp_path / "home", tmp_path / "repo"
    home.mkdir()
    (root / ".git").mkdir(parents=True)
    git_config = '[remote "origin"]\nurl = https://github.com/instruction-fixture/roundtrip.git\n'
    (root / ".git" / "config").write_text(git_config, encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))
    monkeypatch.setenv("GWT_INDEX_FAKE_EMBEDDING", "1")
    monkeypatch.setattr(runner, "_MODEL_CACHE", None)
    monkeypatch.setattr(runner, "_MODEL_CACHE_REVISION", None)
    # Allowlist only executable lookup/platform state, excluding all inherited
    # GWT session, hook forwarding, execution, Git and credential settings.
    child_env = {key: os.environ[key] for key in ("PATH", "SYSTEMROOT", "WINDIR", "PATHEXT") if key in os.environ}
    child_env.update({
        "HOME": str(home), "USERPROFILE": str(home),
        "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_ATTR_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0",
    })

    def invoke(arguments, payload, cwd=root):
        completed = subprocess.run(
            [str(binary), *arguments], input=json.dumps(payload, ensure_ascii=False),
            cwd=cwd, env=child_env, text=True, capture_output=True, timeout=30,
        )
        assert completed.returncode == 0, completed.stderr
        return json.loads(completed.stdout)

    secret = "sk-proj-" + "instructionfixturecredential0123456789"
    original = "報告は日本語で説明してください。\n\n判断根拠も残してください。\n" + secret
    submissions = [
        (original, "報告言語の常設指示", "日本語で根拠を報告します。", ["日本語で報告する"]),
        ("Please report in English and include all decision rationale.",
         "報告言語を英語へ変更", "I will report in English with the rationale.", ["Report in English"]),
    ]
    saved = []
    for text, question, answer, actions in submissions:
        response = invoke([], {
            "schema_version": 1, "operation": "pm.report.post",
            "params": {
                "project_root": str(root), "kind": "qa", "question": question, "body": answer,
                "instruction": {
                    "original": text, "kind": "standing", "actions": actions, "issue_numbers": [5342],
                    "supersedes": [saved[0]["id"]] if saved else [],
                },
            },
        })
        assert response["ok"], response
        payload = json.loads(response["output"])
        assert "instruction" in payload, payload
        assert payload["instruction"]["id"] == payload["report"]["id"]
        assert payload["instruction"]["summary"] == payload["report"]["question"] == question
        assert payload["instruction"]["answer"] == payload["report"]["body"] == answer
        assert secret not in json.dumps(payload)
        saved.append(payload["instruction"])

    sources = list(home.glob(".gwt/projects/*/project-state/user-instructions.json"))
    assert len(sources) == 1
    source = sources[0]
    raw = source.read_text(encoding="utf-8")
    assert secret not in raw
    records = json.loads(raw)
    assert records == [dict(saved[0], superseded_by=saved[1]["id"]), saved[1]]
    assert records[0]["original"] == original.replace(secret, "***redacted***")
    repo_hash = source.parents[1].name
    db_root = tmp_path / "db"
    indexed = runner.action_index_memory_v2(str(root), repo_hash, None, db_root=db_root)
    assert indexed["ok"] and indexed["indexed"] == 2, indexed
    result = runner.action_search_multi_v2(
        repo_hash, None, str(root), "report language and rationale", 10,
        scopes=["memory"], db_root=db_root,
    )
    assert result["ok"] and result["scopes"]["memory"]["state"] == "fresh", result
    hits = {hit["instruction_id"]: hit for hit in result["memoryResults"]}
    assert set(hits) == {record["id"] for record in records}
    for record in records:
        for field, value in record.items():
            if field != "id":
                assert hits[record["id"]][field] == value
    assert hits[records[0]["id"]]["status"] == "superseded"
    assert hits[records[1]["id"]]["status"] == "active"
    assert secret not in json.dumps(result)

    pm_worktree = source.parents[1] / "pm" / "worktree"
    (pm_worktree / ".git").mkdir(parents=True)
    (pm_worktree / ".git" / "config").write_text(git_config, encoding="utf-8")
    startup = invoke(["hook", "event", "SessionStart"], {}, cwd=pm_worktree)
    context = startup["hookSpecificOutput"]["additionalContext"]
    assert saved[1]["id"] in context and saved[1]["summary"] in context
    assert saved[0]["id"] not in context
    assert secret not in context


@pytest.mark.skipif(
    os.environ.get("GWT_INDEX_REAL_EMBEDDING_TEST") != "1",
    reason="opt-in real multilingual-e5 semantic integration",
)
def test_real_multilingual_search_finds_instruction_without_shared_keywords(instruction_corpus, monkeypatch):
    monkeypatch.setenv("GWT_INDEX_FAKE_EMBEDDING", "0")
    monkeypatch.setenv("HF_HOME", str(REAL_MODEL_CACHE))
    monkeypatch.setattr(runner, "_MODEL_CACHE", None)
    monkeypatch.setattr(runner, "_MODEL_CACHE_REVISION", None)
    _, _, source, _, records = instruction_corpus
    records[0].update(
        original="変更を公開する前に必ず自動テストで動作を確かめてください。",
        summary="公開前の動作確認", answer="動作確認してから公開します。", actions=["自動検証を実行する"],
        superseded_by=None,
    )
    records[1].update(
        original="Always explain the reasoning behind the selected solution in the final report.",
        summary="Decision rationale in reports", answer="I will include the reasons.", actions=["Document decisions"],
    )
    source.write_text(json.dumps(records, ensure_ascii=False), encoding="utf-8")
    index_corpus(instruction_corpus)
    for query, expected in (
        ("Verify correctness before publishing the changes", "instruction-old"),
        ("最終報告では、結論だけでなく意思決定の背景も示してほしい", "instruction-new"),
    ):
        hits = search_multi_corpus(instruction_corpus, query, n_results=1)["memoryResults"]
        assert hits[0].get("instruction_id") == expected, hits
