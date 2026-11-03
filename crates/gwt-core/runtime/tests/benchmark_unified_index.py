"""Compare isolated Issue/SPEC indexes against a Git baseline.

Run with the runner's Python environment, for example:
    python crates/gwt-core/runtime/tests/benchmark_unified_index.py

Uses the cached real E5 model offline by default; --embedding fake measures
storage/control-flow overhead only, not model speed or retrieval quality.
All writable state lives in a newly created directory under target/.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[4]
RUNNER = ROOT / "crates/gwt-core/runtime/chroma_index_runner.py"
REPO_HASH = "benchmark5562"
QUERY = "semantic index repair and cache refresh"


def seed_fixture(home: Path) -> str:
    cache = home / ".gwt/cache/issues" / REPO_HASH
    for number in range(1, 10):
        entry = cache / str(number)
        entry.mkdir(parents=True)
        is_spec = number > 6
        title = f"{'Specification' if is_spec else 'Issue'} {number}: index repair"
        meta = {"number": number, "title": title, "state": "open",
                "labels": ["gwt-spec", "phase/implementation"] if is_spec else ["bug"]}
        (entry / "meta.json").write_text(json.dumps(meta, sort_keys=True), encoding="utf-8")
        body = f"{title}\nSemantic index cache refresh must preserve available search results.\n"
        (entry / "body.md").write_text(body, encoding="utf-8")
        if is_spec:
            (entry / "sections").mkdir()
            sections = "\n\n".join(
                f"## {heading}\nSpecification {number} requires {detail}."
                for heading, detail in (
                    ("Background", "one semantic corpus for cached Issues and SPECs"),
                    ("Requirements", "full section content and a shared index repair lifecycle"),
                    ("Acceptance", "cache refresh followed by successful semantic search"),
                    ("Operations", "atomic publication while the previous index serves queries"),
                )
            )
            (entry / "sections/spec.md").write_text(sections, encoding="utf-8")
    (cache / "source-revision").write_text(
        "00000000-0000-4000-8000-000000005562", encoding="utf-8"
    )
    digest = hashlib.sha256()
    for path in sorted(cache.rglob("*")):
        if path.is_file():
            digest.update(path.relative_to(cache).as_posix().encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()


def load_runner(path: Path):
    spec = importlib.util.spec_from_file_location("benchmark_runner", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def worker(args) -> dict:
    runner = load_runner(args.runner)
    runner.configure_qos_threads(
        "background" if args.worker == "build" else "interactive",
        single_query=args.worker == "search",
    )
    db_root = args.state / "index"
    project = args.state / "project"
    project.mkdir(exist_ok=True)
    if args.worker == "build":
        actions = [lambda: runner.action_index_issues_v2(
            repo_hash=REPO_HASH, project_root=str(project), db_root=db_root,
            respect_ttl=False, qos="background",
        )]
        if args.variant == "baseline":
            actions.append(lambda: runner.action_index_specs_v2(
                project_root=str(project), repo_hash=REPO_HASH,
                worktree_hash=None, db_root=db_root, qos="background",
            ))
        builds = []
        for action in actions:
            started = time.perf_counter()
            result = action()
            elapsed = (time.perf_counter() - started) * 1000
            if not result.get("ok"):
                raise RuntimeError(f"index build failed: {result}")
            builds.append({"wall_ms": elapsed, "result": result})
        counts = {}
        for scope in ("issues", "specs") if args.variant == "baseline" else ("issues",):
            db_path = runner.resolve_db_path(REPO_HASH, None, scope, db_root=db_root)
            client, collection = runner._open_chroma_collection(
                runner.resolve_active_store(db_path), scope
            )
            try:
                counts[scope] = collection.count()
            finally:
                runner._close_chroma_client(client)
        return {"build_wall_ms": sum(item["wall_ms"] for item in builds),
                "builds": builds, "records_by_store": counts,
                "record_count": sum(counts.values())}

    searches = []
    for temperature in ("cold", "warm"):
        started = time.perf_counter()
        result = runner.action_search_multi_v2(
            repo_hash=REPO_HASH, worktree_hash=None, project_root=str(project),
            query=QUERY, n_results=5, scopes=["specs", "issues"], db_root=db_root,
        )
        elapsed = (time.perf_counter() - started) * 1000
        if not result.get("ok"):
            raise RuntimeError(f"search failed: {result}")
        for scope in ("specs", "issues"):
            result_key = runner._scope_result_key(scope)
            if not result.get("scope_results", {}).get(scope, {}).get(result_key):
                raise RuntimeError(f"search returned no results for {scope}: {result}")
        searches.append({"temperature": temperature, "wall_ms": elapsed, "result": result})
    return {"searches": searches}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-ref", default="e1d040beb")
    parser.add_argument("--embedding", choices=("real", "fake"), default="real")
    parser.add_argument("--output", type=Path, default=ROOT / "target/unified-index-benchmark.json")
    parser.add_argument("--worker", choices=("build", "search"), help=argparse.SUPPRESS)
    parser.add_argument("--variant", choices=("baseline", "unified"), help=argparse.SUPPRESS)
    parser.add_argument("--runner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--state", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.worker:
        result = worker(args)
        print(json.dumps(result, ensure_ascii=True))
        return

    args.output.parent.mkdir(parents=True, exist_ok=True)
    run_root = Path(tempfile.mkdtemp(prefix="unified-index-benchmark-", dir=args.output.parent))
    baseline = subprocess.check_output(
        ["git", "show", f"{args.baseline_ref}:crates/gwt-core/runtime/chroma_index_runner.py"],
        cwd=ROOT,
    )
    sources = {"baseline": baseline, "unified": RUNNER.read_bytes()}
    hf_home = os.environ.get("HF_HOME", str(Path.home() / ".cache/huggingface"))
    hf_cache = Path(os.environ.get("HF_HUB_CACHE", os.environ.get("HUGGINGFACE_HUB_CACHE", str(Path(hf_home) / "hub"))))
    model_ref = hf_cache / "models--intfloat--multilingual-e5-base/refs/main"
    report = {
        "baseline_ref": args.baseline_ref,
        "baseline_sha": subprocess.check_output(
            ["git", "rev-parse", args.baseline_ref], cwd=ROOT, text=True
        ).strip(),
        "checkout_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "python": sys.version,
        "embedding": args.embedding,
        "model_id": "intfloat/multilingual-e5-base" if args.embedding == "real" else "deterministic-fake-32d",
        "cached_model_revision": model_ref.read_text().strip() if args.embedding == "real" and model_ref.is_file() else None,
        "embedding_note": "Cached real model, offline." if args.embedding == "real" else "Fake vectors measure storage/control flow only, not model speed or retrieval quality.",
        "fixture": {"ordinary_issues": 6, "spec_issues": 3, "sections_per_spec": 4},
        "query": QUERY,
        "measurement": {
            "qos": "Build background (2 threads), search interactive single-query (1 thread), matching CLI defaults.",
            "build": "Sum of index API wall times in one fresh process per variant; excludes fixture seeding and record counting.",
            "search": "Both scopes in a fresh process (cold model/store), then repeated in that process (warm); OS file cache is uncontrolled.",
            "index_bytes": "All regular files under the isolated index root after build process exit, before search.",
            "limits": "Small synthetic fixture; one sample per variant, host load is uncontrolled; no retrieval-quality or production-scale speed claim.",
        },
        "artifacts": str(run_root), "variants": {},
    }
    for variant, source in sources.items():
        state = run_root / variant
        state.mkdir()
        home = state / "home"
        fixture_digest = seed_fixture(home)
        runner_path = state / "runner.py"
        runner_path.write_bytes(source)
        env = os.environ.copy()
        env.update({"HOME": str(home), "USERPROFILE": str(home), "HF_HOME": hf_home,
                    "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1",
                    "GWT_INDEX_COORDINATOR_ROOT": str(state / "coordinator"),
                    "ANONYMIZED_TELEMETRY": "False",
                    "GWT_INDEX_FAKE_EMBEDDING": "1" if args.embedding == "fake" else "0"})
        item = {"runner_sha256": hashlib.sha256(source).hexdigest(), "fixture_sha256": fixture_digest}
        for phase in ("build", "search"):
            completed = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), "--worker", phase,
                 "--variant", variant, "--runner", str(runner_path), "--state", str(state)],
                env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=600,
            )
            (state / f"{phase}.stdout.log").write_text(completed.stdout, encoding="utf-8")
            (state / f"{phase}.stderr.log").write_text(completed.stderr, encoding="utf-8")
            if completed.returncode:
                raise RuntimeError(f"{variant} {phase} failed; see {state / (phase + '.stderr.log')}")
            item.update(json.loads(completed.stdout.splitlines()[-1]))
            if phase == "build":
                item["index_bytes"] = sum(p.stat().st_size for p in (state / "index").rglob("*") if p.is_file())
            report["variants"][variant] = item
            args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps({"output": str(args.output), "embedding": args.embedding,
                      "variants": {key: {metric: value[metric] for metric in
                                        ("index_bytes", "record_count", "build_wall_ms")}
                                   for key, value in report["variants"].items()}}))


if __name__ == "__main__":
    main()
