#!/usr/bin/env python3
"""Opt-in, path-scoped application writer diagnostics for Issue #6979.

Chroma payload bytes are logical bytes at its normalized native API boundary,
not SQLite/WAL/HNSW or OS write bytes. Python file writes are reported separately.
Nothing is installed in ordinary runtimes. Use --help for isolated invocation.
"""

from __future__ import annotations

import argparse
import builtins
import datetime as dt
import functools
import gzip
import hashlib
import io
import json
import math
import os
from pathlib import Path
import sys
import tempfile
import threading
import time


class WriteDiagnostics:
    def __init__(self, root: Path, output: Path):
        self.root = root.resolve()
        self.output = output.resolve()
        if self.output.is_relative_to(self.root):
            raise ValueError("diagnostic output must be outside the measured index root")
        self.output.parent.mkdir(parents=True, exist_ok=True)
        self._open = io.open
        self._lock = threading.Lock()
        self._chroma_patched = False

    def record(self, path, category: str, components: dict[str, int]) -> None:
        if isinstance(path, int):
            return
        path = Path(path).resolve()
        if not path.is_relative_to(self.root):
            return
        event = {"schema_version": 1, "pid": os.getpid(), "time_ns": time.time_ns(),
                 "path": path.relative_to(self.root).as_posix(), "category": category,
                 "bytes": sum(components.values()), "components": components}
        # Capture no document, metadata, vector or credential contents.
        with self._lock, self._open(self.output, "a", encoding="utf-8", newline="\n") as output:
            output.write(json.dumps(event, separators=(",", ":")) + "\n")

    def patch_chroma(self) -> None:
        if self._chroma_patched:
            return
        from chromadb.api.rust import RustBindingsAPI

        original = RustBindingsAPI._upsert

        @functools.wraps(original)
        def upsert(api, *args, **kwargs):
            import inspect

            values = inspect.signature(original).bind(api, *args, **kwargs).arguments
            root = api._system.settings.require("persist_directory")
            components = {}
            if Path(root).resolve().is_relative_to(self.root):
                vectors = values.get("embeddings")
                # Legacy copy paths pass Python lists directly to this API;
                # its persisted vector contract is float32 in either shape.
                components["embedding_float32"] = (
                    sum(4 * len(vector) for vector in vectors) if vectors is not None else 0
                )
                for key in ("ids", "documents", "uris"):
                    components[key + "_utf8"] = sum(
                        len(value.encode("utf-8")) for value in (values.get(key) or []) if value is not None
                    )
                components["metadata_canonical_json_utf8"] = sum(
                    len(json.dumps(value, sort_keys=True, ensure_ascii=False,
                                   separators=(",", ":"), allow_nan=False).encode("utf-8"))
                    for value in (values.get("metadatas") or []) if value is not None
                )
            result = original(api, *args, **kwargs)
            if components:
                self.record(root, "chroma_logical_payload", components)
            return result

        RustBindingsAPI._upsert = upsert
        self._chroma_patched = True

    def install(self) -> None:
        original_import = builtins.__import__
        original_open = self._open
        diagnostic = self

        class Writer:
            def __init__(self, stream, path, newline):
                self.stream, self.path, self.newline = stream, path, newline

            def __getattr__(self, name):
                return getattr(self.stream, name)

            def __enter__(self):
                self.stream.__enter__()
                return self

            def __exit__(self, *args):
                return self.stream.__exit__(*args)

            def write(self, value):
                written = self.stream.write(value)
                data = value[:written]
                if isinstance(data, str):
                    replacement = os.linesep if self.newline is None else self.newline
                    if replacement:
                        data = data.replace("\n", replacement)
                    data = data.encode(self.stream.encoding, self.stream.errors)
                diagnostic.record(self.path, "python_file_write", {"data": len(data)})
                return written

            def writelines(self, lines):
                for line in lines:
                    self.write(line)

        @functools.wraps(original_open)
        def counted_open(file, mode="r", *args, **kwargs):
            stream = original_open(file, mode, *args, **kwargs)
            if not isinstance(file, int) and any(flag in mode for flag in "wax+"):
                if Path(file).resolve().is_relative_to(self.root):
                    newline = args[3] if len(args) > 3 else kwargs.get("newline")
                    return Writer(stream, file, newline)
            return stream

        @functools.wraps(original_import)
        def counted_import(name, *args, **kwargs):
            module = original_import(name, *args, **kwargs)
            if (name == "chromadb" and hasattr(module, "PersistentClient")
                    and hasattr(sys.modules.get("chromadb.api.rust"), "RustBindingsAPI")):
                self.patch_chroma()
            return module

        io.open = counted_open
        builtins.open = counted_open
        builtins.__import__ = counted_import
        if "chromadb.api.rust" in sys.modules:
            self.patch_chroma()


def enable_from_environment() -> None:
    WriteDiagnostics(Path(os.environ["GWT_DIAGNOSTIC_INDEX_ROOT"]),
                     Path(os.environ["GWT_DIAGNOSTIC_INDEX_WRITES"])).install()


def self_test(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="validation-", dir=directory))
    root = directory / "index"
    root.mkdir(parents=True)
    output = directory / "writes.jsonl"
    if output.exists():
        raise ValueError("self-test requires a fresh directory")
    diagnostics = WriteDiagnostics(root, output)
    diagnostics.install()
    marker = b"x" * (1024 * 1024)
    (root / "marker.bin").write_bytes(marker)
    (directory / "excluded.bin").write_bytes(marker)
    import chromadb
    import numpy as np

    client = chromadb.PersistentClient(path=str(root / "chroma"))
    collection = client.get_or_create_collection("diagnostic", embedding_function=None)
    collection.upsert(ids=["marker"], documents=[marker.decode("ascii")],
                      embeddings=np.array([[1, 0, 0, 0]], dtype=np.float32), metadatas=[{"kind": "marker"}])
    client._server._upsert(collection_id=collection.id, ids=["list-marker"],
                           embeddings=[[1.0, 0.0, 0.0, 0.0]])
    found = collection.get(ids=["marker"], include=["documents", "embeddings"])
    assert len(found["documents"][0].encode("utf-8")) == len(marker)
    assert found["embeddings"][0].shape == (4,)
    events = [json.loads(line) for line in output.read_text(encoding="utf-8").splitlines()]
    direct = [event for event in events if event["category"] == "python_file_write"]
    native = [event for event in events if event["category"] == "chroma_logical_payload"]
    assert len(direct) == 1 and direct[0]["path"] == "marker.bin" and direct[0]["bytes"] == len(marker), direct
    expected = len(marker) + 16 + len("marker") + len(b'{"kind":"marker"}')
    assert len(native) == 2 and native[0]["path"] == "chroma" and native[0]["bytes"] == expected, native
    assert native[1]["bytes"] == 16 + len("list-marker"), native
    print(json.dumps({"result": "PASS", "direct_marker_bytes": len(marker),
                      "chroma_document_bytes": len(marker), "chroma_total_logical_bytes": expected,
                      "output": str(output), "physical_os_writes": "unmeasured"}), flush=True)


def check_results(directory: Path) -> None:
    """Recompute the owner's gates from the published, immutable raw artifacts."""
    profiles = {}
    for profile in ("before", "after"):
        base = directory / (profile + "-hour")
        inventory = json.loads((base / "artifacts.json").read_text(encoding="utf-8"))
        raw = {}
        for entry in inventory["artifacts"]:
            stored = (base / entry["artifact"]).read_bytes()
            assert len(stored) == entry["stored_bytes"]
            assert hashlib.sha256(stored).hexdigest() == entry["stored_sha256"]
            data = gzip.decompress(stored) if entry["artifact"].endswith(".gz") else stored
            assert len(data) == entry["raw_bytes"]
            assert hashlib.sha256(data).hexdigest() == entry["raw_sha256"]
            raw[entry["raw_name"]] = data.decode("utf-8")
        load = lambda name: json.loads(raw[name])
        rows = lambda name: [json.loads(line) for line in raw[name].splitlines() if line.strip()]
        summary = load("summary.json")
        window = load("hour-analysis.json")["measured_window"]
        begin, end = window["started_at_ms"], window["completed_at_ms"]
        assert begin == load("measurement-window.json")["started_at_ms"]
        assert end - begin >= 3_600_000 and summary["source_events"] == 13
        source = load("trace-source-diff.json")
        assert not source["unexpected_issue_record_changes"]
        assert source["changed_project_files"] == [source["expected_file"]]
        events = rows("events.jsonl")
        assert [event["event"] for event in events] == list(range(13))
        assert [event["scheduled_offset_seconds"] for event in events] == list(range(0, 3241, 270))
        initial_records = load("input-record-fingerprints-before.json")
        final_records = load("input-record-fingerprints-after.json")
        assert initial_records["record_count"] == final_records["record_count"] == 5889
        initial_records, final_records = initial_records["record_fingerprints"], final_records["record_fingerprints"]
        changed = {key for key in set(initial_records) | set(final_records)
                   if initial_records.get(key) != final_records.get(key)}
        assert changed == {str(events[0]["issue_number"])}
        writes = [row for row in rows("writes.jsonl") if begin <= row["time_ns"] / 1e6 <= end]
        assert any(row["category"] == "python_file_write" and row["bytes"] > 0 for row in writes)
        assert any(row["category"] == "chroma_logical_payload" and "/issues" in row["path"] for row in writes)
        assert any(row["category"] == "chroma_logical_payload" and "/file-index-v2/" in row["path"] for row in writes)
        if profile == "before":
            actions = rows("baseline-actions.jsonl")
            assert len(actions) == 26 and all(row["exit_code"] == 0 and row["result"]["ok"] for row in actions)
            assert load("corrected-instrumentation.json")["all_26_actions_have_exact_native_vector_coverage"]
            durations = [row["wall_ms"] for row in actions]
        else:
            assert summary["failure"] is None and summary["writer_instrumentation_proven"]
            assert len(load("source-to-completion.json")) == 13
            assert all(row["all_observed_targets_complete"] for row in load("source-to-completion.json"))
            native = [row for row in rows("native-events.jsonl")
                      if begin <= dt.datetime.fromisoformat(row["timestamp"].replace("Z", "+00:00")).timestamp()*1000 <= end]
            terminal = {"project index refresh claim completed", "project index refresh checkpointed for continuation",
                        "project index refresh claim failed"}
            claims = [row["fields"] for row in native if row.get("target") == "gwt::index"
                      and row["fields"].get("message") in terminal]
            assert claims and not any(row["message"] == "project index refresh claim failed" for row in claims)
            durations = [row["elapsed_ms"] for row in claims]
            assert sorted(durations) == sorted(load("claim-metrics.json")["all_claim_elapsed_ms"])
            for row in native:
                if row.get("target") != "gwt.frontend.timing":
                    continue
                fields = row["fields"]
                message = fields.get("message", "")
                assert fields.get("receive_to_complete_ms", 0) < 100
                assert message != "frontend dispatch still running"
                if message == "GUI event loop dispatch handled" or " blocked the GUI event loop for " in message:
                    assert fields["elapsed_ms"] < 100
            for theme in ("dark", "light"):
                ui = load(f"ui-{theme}-summary.json")
                assert not ui["errors"] and not ui["failure"] and ui["sample_count"] > 0
                assert all(chunk["dropped_entries"] == 0 for chunk in ui["trace_chunks"])
                assert all(row["elapsed_ms"] <= 1000 for row in rows(f"ui-{theme}.jsonl"))
            scans = rows("scan-completions.jsonl")
            assert scans and all(not row["last_scan_error"] and row["observed_control_to_commit_ms"] < 60000 for row in scans)
            overlap = load("overlap-evidence.json")
            assert overlap["scan_refresh_overlap_count"] > 0 and overlap["ui_scan_refresh_overlap_count"] > 0
        profiles[profile] = {"bytes": sum(row["bytes"] for row in writes),
                             "p95_ms": sorted(durations)[math.ceil(len(durations)*.95)-1],
                             "initial_records": initial_records, "final_records": final_records,
                             "trace": [{key: value for key, value in event.items()
                                        if key not in {"changed_at_ms", "mutation_finished_at_ms"}} for event in events]}
    assert profiles["before"]["trace"] == profiles["after"]["trace"]
    assert profiles["before"]["initial_records"] == profiles["after"]["initial_records"]
    assert profiles["before"]["final_records"] == profiles["after"]["final_records"]
    assert profiles["after"]["bytes"] * 10 <= profiles["before"]["bytes"]
    assert profiles["after"]["p95_ms"] <= 5000
    print(json.dumps({"result": "PASS", "before_application_writer_bytes": profiles["before"]["bytes"],
                      "after_application_writer_bytes": profiles["after"]["bytes"],
                      "after_refresh_p95_ms": profiles["after"]["p95_ms"], "physical_os_writes": "unmeasured"}))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--self-test", type=Path, help="fresh directory for direct/real-Chroma 1MiB controls")
    modes.add_argument("--install-site", type=Path, help="create opt-in sitecustomize in an isolated diagnostics directory")
    modes.add_argument("--check-results", type=Path, help="recompute one-hour acceptance from published before/after raw artifacts")
    args = parser.parse_args()
    if args.self_test:
        self_test(args.self_test.resolve())
    elif args.check_results:
        check_results(args.check_results.resolve())
    else:
        args.install_site.mkdir(parents=True, exist_ok=True)
        path = str(Path(__file__).resolve())
        (args.install_site / "sitecustomize.py").write_text(
            "import importlib.util, os\n"
            "if os.environ.get('GWT_DIAGNOSTIC_INDEX_ROOT') and os.environ.get('GWT_DIAGNOSTIC_INDEX_WRITES'):\n"
            f"    spec = importlib.util.spec_from_file_location('gwt_write_diagnostics', {path!r})\n"
            "    module = importlib.util.module_from_spec(spec)\n"
            "    spec.loader.exec_module(module)\n"
            "    module.enable_from_environment()\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
