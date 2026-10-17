#!/usr/bin/env python3
"""Measure the real Rust search/admission/worker path in an isolated HOME.

Prepare once with an unmodified baseline binary, then measure baseline and
after against that same fixed corpus. No fake embeddings or production leases
are used. Cold bursts are retained even when they fail the latency target.
Use Python 3.12+ to drive the Windows harness. Run --help for the required explicit paths; dependency-python must already have
the requirements installed and the real E5 model must already be cached.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import threading
import time
import uuid


SCOPES = ["issues", "specs", "memory"]
QUERY = "search admission index sharing repair performance"


def save(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def digest(path: Path) -> dict:
    return {"path": str(path), "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def run(argv: list[str], env: dict, cwd: Path, timeout: float, stdin: str | None = None) -> dict:
    started = time.perf_counter()
    process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", errors="replace", cwd=cwd, env=env,
                               creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
                               start_new_session=os.name != "nt")
    try:
        stdout, stderr = process.communicate(stdin, timeout=timeout)
        return {"wall_ms": (time.perf_counter() - started) * 1000,
                "exit_code": process.returncode, "stdout": stdout, "stderr": stderr,
                "timed_out": False}
    except subprocess.TimeoutExpired:
        elapsed_ms = (time.perf_counter() - started) * 1000
        # Kill only this owned process tree, including Python launchers; leaving
        # an orphan model process would contaminate subsequent phases.
        if os.name == "nt":
            subprocess.run(["taskkill.exe", "/PID", str(process.pid), "/T", "/F"],
                           capture_output=True, timeout=30)
        else:
            os.killpg(process.pid, signal.SIGKILL)
        stdout, stderr = process.communicate(timeout=30)
        return {"wall_ms": (time.perf_counter() - started) * 1000,
                "exit_code": None, "stdout": stdout, "stderr": stderr,
                "timed_out": True, "timeout_observed_ms": elapsed_ms}


def envelope(binary: Path, operation: str, params: dict, env: dict, cwd: Path, timeout: float) -> dict:
    result = run([str(binary)], env, cwd, timeout,
                 json.dumps({"schema_version": 1, "operation": operation, "params": params}) + "\n")
    try:
        result["response"] = json.loads(result["stdout"])
        if isinstance(result["response"].get("output"), str):
            result["output"] = json.loads(result["response"]["output"])
    except (ValueError, TypeError, AttributeError):
        pass
    return result


def isolated_env(home: Path, cache: Path, coordinator: Path) -> dict:
    # Production launch/session bindings and daemon endpoints must not leak into
    # a benchmark. Retain normal PATH and OS variables for the installed Python.
    env = {key: value for key, value in os.environ.items() if not key.startswith("GWT_")}
    env.update({"HOME": str(home), "USERPROFILE": str(home),
                "GWT_DISABLE_BACKGROUND_INDEX": "1", "GWT_INDEX_FAKE_EMBEDDING": "0",
                "GWT_INDEX_COORDINATOR_ROOT": str(coordinator), "HF_HOME": str(cache),
                "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1",
                "PYTHONUTF8": "1",
                "GIT_TERMINAL_PROMPT": "0", "GH_PROMPT_DISABLED": "1"})
    return env


def host_snapshot(env: dict, cwd: Path) -> dict:
    result = {"captured_utc": dt.datetime.now(dt.timezone.utc).isoformat(), "logical_cpus": os.cpu_count()}
    if os.name == "nt":
        command = "$memory = Get-CimInstance Win32_OperatingSystem; @{free_memory_kib=$memory.FreePhysicalMemory; total_memory_kib=$memory.TotalVisibleMemorySize; cpu_load_percent=@(Get-CimInstance Win32_Processor | Select-Object -ExpandProperty LoadPercentage)} | ConvertTo-Json -Compress"
        probe = run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command], env, cwd, 30)
        try:
            result.update(json.loads(probe["stdout"]))
        except ValueError:
            result["probe"] = probe
    else:
        result["load_average"] = os.getloadavg()
    return result


def source_provenance(args, runner: Path) -> dict:
    source_paths = ["crates/gwt/src/index_search.rs", "crates/gwt/src/index_worker.rs",
                    "crates/gwt-core/runtime/chroma_index_runner.py",
                    "crates/gwt-core/src/index_coordinator.rs", "crates/gwt/src/bin/gwtd.rs",
                    "crates/gwt/src/cli.rs", "crates/gwt/src/cli/json_envelope.rs"]
    def git(*arguments):
        return subprocess.check_output(["git", *arguments], cwd=args.checkout).decode("utf-8").strip()
    declared_runner = subprocess.check_output(["git", "show",
        f"{args.source_ref}:crates/gwt-core/runtime/chroma_index_runner.py"], cwd=args.checkout)
    return {"checkout_head": git("rev-parse", "HEAD"),
            "declared_source_ref_resolved": git("rev-parse", args.source_ref),
            "embedded_runner_matches_declared_ref_normalized":
                runner.read_bytes().replace(b"\r\n", b"\n") == declared_runner.replace(b"\r\n", b"\n"),
            "embedded_runner_matches_current_worktree": runner.read_bytes() == (args.checkout / source_paths[2]).read_bytes(),
            "current_worktree_sources": [digest(args.checkout / path) for path in source_paths],
            "current_worktree_source_status": git("status", "--porcelain", "--", *source_paths),
            "note": "Working-tree Rust digests are informational; the executable digest identifies what was measured."}


def materialize(args, env: dict) -> tuple[Path, Path, dict]:
    runtime = args.home / ".gwt/runtime"
    runtime.mkdir(parents=True, exist_ok=True)
    binary_bytes = args.binary.read_bytes()
    # Prewrite the exact requirements to prevent runtime ensure from attempting
    # an install into the shared, already-installed dependency environment.
    requirements = (args.checkout / "crates/gwt-core/runtime/project_index_requirements.txt").read_bytes()
    # include_str! preserves checkout line endings; git-show bytes can have a
    # different hash on Windows. Fail before runtime ensure if these are not
    # the measured binary's embedded requirements.
    if requirements not in binary_bytes:
        raise RuntimeError("Checkout requirements do not match the measured binary's embedded bytes")
    (runtime / "project_index_requirements.txt").write_bytes(requirements)
    requirements_hash = hashlib.sha256(requirements).hexdigest()[:16]
    private_venv = runtime / "venvs" / f"chroma-venv-{requirements_hash}"
    dependency_root = args.dependency_python.parent.parent.resolve()
    # The runtime may delete/rebuild its venv after a failed probe. Its directory
    # must therefore be private, even when installed dependencies are reused.
    # A .pth adds read-only dependency imports; it never redirects the venv root.
    if private_venv.is_symlink() or (os.name == "nt" and private_venv.is_junction()):
        if not private_venv.absolute().is_relative_to(args.home):
            raise RuntimeError("Dependency link escaped the isolated HOME")
        if private_venv.is_symlink():
            private_venv.unlink()
        else:
            os.rmdir(private_venv)  # Remove only the junction, never its target.
    if not private_venv.exists():
        scripts = private_venv / ("Scripts" if os.name == "nt" else "bin")
        scripts.mkdir(parents=True)
        shutil.copy2(dependency_root / "pyvenv.cfg", private_venv / "pyvenv.cfg")
        shutil.copy2(args.dependency_python, scripts / args.dependency_python.name)
        dependency_site = (dependency_root / "Lib/site-packages" if os.name == "nt" else
                           next(dependency_root.glob("lib/python*/site-packages")))
        private_site = private_venv / dependency_site.relative_to(dependency_root)
        private_site.mkdir(parents=True)
        (private_site / "issue-5598-dependencies.pth").write_text(
            "import site; site.addsitedir(" + repr(str(dependency_site)) + ")\n", encoding="utf-8")
    elif private_venv.resolve() == dependency_root:
        raise RuntimeError("The benchmark venv root must be private")
    # Reuse a successful runtime probe only when the materialized runner bytes
    # really belong to this executable. Avoid repeating the all-scope status
    # scan merely to reuse already verified runtime assets.
    try:
        existing = json.loads((runtime / "project_index_runtime_manifest.json").read_text(encoding="utf-8"))
        probe = json.loads((runtime / "probe_cache.json").read_text(encoding="utf-8"))
        existing_runner = runtime / existing["runner"]["path"]
        reused = (existing_runner.read_bytes() in binary_bytes
                  and probe["runner_hash"] == existing["runner"]["sha256_16"]
                  and probe["requirements_hash"] == requirements_hash)
    except (OSError, ValueError, KeyError):
        reused = False
    result = ({"reused_matching_embedded_assets": True, "probe_cache": probe} if reused else
              envelope(args.binary, "index.status", {}, env, args.checkout, 180))
    save(args.output / f"{args.label}-runtime-status.json", result)
    manifest = json.loads((runtime / "project_index_runtime_manifest.json").read_text(encoding="utf-8"))
    python = runtime / manifest["venv"]["path"] / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    runner = runtime / manifest["runner"]["path"]
    # An unavailable unrelated file scope need not invalidate these three
    # explicitly built repo scopes. A failed runtime probe does invalidate them.
    if not python.is_file() or not runner.is_file():
        raise RuntimeError("The checkout binary did not materialize its search runtime")
    probe = json.loads((runtime / "probe_cache.json").read_text(encoding="utf-8"))
    if probe.get("runner_hash") != manifest["runner"]["sha256_16"] or runner.read_bytes() not in binary_bytes:
        raise RuntimeError("The checkout binary's runtime probe did not complete successfully")
    return python, runner, manifest


def reset_coordinator(home: Path, label: str) -> dict:
    root = home / ".gwt/runtime/index-coordinator"
    if not root.resolve().is_relative_to(home.resolve()):
        raise RuntimeError("Coordinator escaped the isolated HOME")
    archived = root.with_name(f"index-coordinator-{label}-archived-{time.time_ns()}")
    if root.exists():
        root.rename(archived)
    root.mkdir(parents=True)
    return {"root": str(root), "archived_previous": str(archived) if archived.exists() else None}


def prepare(args, env: dict) -> None:
    if args.home.exists() and not args.resume_prepare:
        raise RuntimeError("--prepare requires a new isolated --home")
    cache = args.home / ".gwt/cache/issues" / args.repo_hash
    project = args.home / ".gwt/projects" / args.repo_hash
    if args.resume_prepare:
        metadata = json.loads((args.output / "fixture.json").read_text(encoding="utf-8"))
        if Path(metadata["home"]).resolve() != args.home or not cache.is_dir():
            raise RuntimeError("Resumed fixture does not match the isolated HOME")
    else:
        shutil.copytree(args.source_home / ".gwt/cache/issues" / args.repo_hash, cache)
        memory = args.source_home / ".gwt/projects" / args.repo_hash / "work-notes/memory.md"
        (project / "work-notes").mkdir(parents=True)
        shutil.copy2(memory, project / "work-notes/memory.md")
        instructions = args.source_home / ".gwt/projects" / args.repo_hash / "project-state/user-instructions.json"
        if instructions.is_file():
            (project / "project-state").mkdir(parents=True, exist_ok=True)
            shutil.copy2(instructions, project / "project-state/user-instructions.json")
        source_files = sorted(path for path in cache.rglob("*") if path.is_file())
        tree_hash = hashlib.sha256()
        for path in source_files:
            tree_hash.update(str(path.relative_to(cache)).replace("\\", "/").encode())
            tree_hash.update(hashlib.sha256(path.read_bytes()).digest())
        metadata = {"captured_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                    "repo_hash": args.repo_hash, "cache_file_count": len(source_files),
                    "cache_tree_sha256": tree_hash.hexdigest(), "memory": digest(project / "work-notes/memory.md"),
                    "source_home": str(args.source_home), "home": str(args.home),
                    "source_ref": args.source_ref, "builds": []}
    python, runner, manifest = materialize(args, env)
    metadata.update({"runtime_manifest": manifest, "binary": digest(args.binary)})
    if args.seed_from_active_index:
        index = args.home / ".gwt/index" / args.repo_hash
        if not index.resolve().is_relative_to(args.home):
            raise RuntimeError("Fixture index escaped the isolated HOME")
        if index.exists():
            archive = index.with_name(f"{args.repo_hash}-aborted-preparation-{time.time_ns()}")
            index.rename(archive)
            metadata.setdefault("aborted_preparation_indices", []).append(str(archive))
        index.mkdir(parents=True)
        source_index = args.source_home / ".gwt/index" / args.repo_hash
        metadata["seed_generations"] = []
        for scope in args.scopes:
            pointer_path = source_index / f"{scope}.gen/active.json"
            pointer = json.loads(pointer_path.read_text(encoding="utf-8"))
            generation = source_index / f"{scope}.gen" / pointer["generation"]
            if generation.parent != source_index / f"{scope}.gen":
                raise RuntimeError("Invalid active generation")
            source_db = generation / "chroma.sqlite3"
            source_stat = source_db.stat()
            shutil.copytree(generation, index / f"{scope}.gen" / pointer["generation"])
            if source_db.stat().st_mtime_ns != source_stat.st_mtime_ns:
                raise RuntimeError("Active generation changed during fixture copy")
            shutil.copy2(pointer_path, index / f"{scope}.gen/active.json")
            (index / scope).mkdir()
            for source, destination in (
                    (source_index / scope / "meta.json", index / scope / "meta.json"),
                    (source_index / f"manifest-{scope}.json", index / f"manifest-{scope}.json")):
                if source.is_file():
                    shutil.copy2(source, destination)
            metadata["seed_generations"].append({"scope": scope, "pointer": pointer, "source_db": digest(source_db)})
    mode = "incremental" if args.seed_from_active_index else "full"
    save(args.output / "fixture.json", metadata)
    for scope in args.scopes:
        result = run([str(python), str(runner), "--action", f"index-{scope}",
                      "--repo-hash", args.repo_hash, "--project-root", str(args.checkout),
                      "--mode", mode, "--qos", "interactive"], env, args.checkout, args.build_timeout)
        try:
            result["output"] = json.loads(result["stdout"].strip().splitlines()[-1])
        except (ValueError, IndexError):
            pass
        metadata["builds"].append({"scope": scope, **result})
        save(args.output / "fixture.json", metadata)
        if result["exit_code"] != 0 or not result.get("output", {}).get("ok"):
            raise RuntimeError(f"Real {scope} index build failed; see fixture.json")


def stop_owned_workers(home: Path) -> list[dict]:
    stopped = []
    for endpoint in (home / ".gwt/runtime/search-workers").glob("*/endpoint.json"):
        try:
            pid = json.loads(endpoint.read_text(encoding="utf-8"))["pid"]
        except (OSError, ValueError, KeyError):
            continue
        worker_dir = str(endpoint.parent.resolve())
        if os.name == "nt":
            command = "& { param($processId, $workerPath) $owned = Get-CimInstance Win32_Process -Filter \"ProcessId = $processId\"; if ($owned -and $owned.CommandLine.Contains($workerPath) -and $owned.CommandLine.Contains('serve-search-worker')) { Stop-Process -Id $owned.ProcessId -Force -ErrorAction Stop; Write-Output 'stopped' } }"
            result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command,
                                     str(pid), worker_dir], capture_output=True, text=True, check=True)
            did_stop = "stopped" in result.stdout
        else:
            try:
                command_line = Path(f"/proc/{pid}/cmdline").read_bytes().decode().replace("\x00", " ")
            except OSError:
                command_line = ""
            did_stop = worker_dir in command_line and "serve-search-worker" in command_line
            if did_stop:
                os.kill(pid, signal.SIGTERM)
        stopped.append({"pid": pid, "worker_dir": worker_dir, "stopped": did_stop})
    return stopped


def retain_local_stdout(args, result: dict) -> None:
    raw = result.pop("stdout")
    raw_path = args.home / ".gwt/diagnostics/issue-5598" / f"{args.label}-{uuid.uuid4().hex}.json"
    raw_path.parent.mkdir(parents=True, exist_ok=True)
    raw_path.write_text(raw, encoding="utf-8")
    result["stdout_local_path"] = str(raw_path)
    result["stdout_sha256"] = hashlib.sha256(raw.encode("utf-8")).hexdigest()


def prewarm_model(args, env: dict, python: Path, runner: Path) -> dict:
    # Match Rust's native worker directory exactly; loading the model through a
    # separate identity would create a second model and would not warm search.
    identity = json.dumps([str(runner), str(python), "0", "interactive"], separators=(",", ":"))
    worker = args.home / ".gwt/runtime/search-workers" / hashlib.sha256(identity.encode()).hexdigest()
    result = run([str(python), str(runner), "--action", "search-multi", "--repo-hash", args.repo_hash,
                  "--project-root", str(args.checkout), "--scopes", ",".join(args.scopes),
                  "--query", QUERY, "--n-results", "3", "--match-mode", "semantic",
                  "--file-index-protocol", "v2", "--no-auto-build", "--qos", "interactive",
                  "--worker-dir", str(worker)], env, args.checkout, args.model_timeout)
    try:
        output = json.loads(result["stdout"])
    except ValueError:
        output = {}
    result["success"] = result["exit_code"] == 0 and output.get("ok") is True
    if result["success"]:
        retain_local_stdout(args, result)
        result["output_projection"] = {key: value for key, value in output.items()
                if not key.endswith("Results") and key != "scope_results"}
        result["result_counts"] = {key: len(value) for key, value in output.items() if key.endswith("Results")}
    return result


def worker_snapshot(args, env: dict, prewarm: dict) -> dict:
    pid = prewarm.get("output_projection", {}).get("worker_pid")
    if os.name != "nt" or not isinstance(pid, int):
        return {"worker_pid": pid, "available": False}
    command = "& { param($workerId, $ownedHome) $worker = Get-CimInstance Win32_Process -Filter \"ProcessId = $workerId\"; $related = @($worker) + @(Get-CimInstance Win32_Process -Filter \"ParentProcessId = $workerId\"); if ($worker) { $related += @(Get-CimInstance Win32_Process -Filter \"ProcessId = $($worker.ParentProcessId)\") }; @($related | Where-Object { $_ -and $_.CommandLine -and $_.CommandLine.Contains($ownedHome) } | ForEach-Object { $live = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; @{pid=$_.ProcessId; parent_pid=$_.ParentProcessId; command_line=$_.CommandLine; priority_class=[string]$live.PriorityClass; priority_value=[int]$live.PriorityClass} }) | ConvertTo-Json -Compress }"
    result = run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command,
                  str(pid), str(args.home)], env, args.checkout, 30)
    try:
        result["processes"] = json.loads(result["stdout"])
    except ValueError:
        pass
    return {"worker_pid": pid, "captured_utc": dt.datetime.now(dt.timezone.utc).isoformat(), **result}


def search(args, env: dict, query: str = QUERY) -> dict:
    started_utc = dt.datetime.now(dt.timezone.utc).isoformat()
    result = envelope(args.binary, "search", {"query": query, "scopes": args.scopes,
                      "match_mode": "semantic", "n_results": 3}, env, args.checkout, args.timeout)
    result["started_utc"] = started_utc
    result["finished_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    result["query"] = query
    output = result.get("output", {})
    result["success"] = (result["exit_code"] == 0 and result.get("response", {}).get("ok") is True
                         and isinstance(output.get("results"), list) and len(output["results"]) > 0
                         and not output.get("unavailable_scopes"))
    result["result_count"] = len(output.get("results", []))
    if result["success"]:
        # Preserve the complete local stdout without publishing unrelated work
        # memory/instruction text in the repository's performance evidence.
        retain_local_stdout(args, result)
        response = result.pop("response")
        result.pop("output", None)
        result["response_projection"] = {key: value for key, value in response.items() if key != "output"}
        result["output_projection"] = {key: output[key] for key in
                ("ok", "stale_scopes", "refresh_queued", "unavailable_scopes") if key in output}
        result["output_projection"]["result_scopes"] = [item.get("scope") for item in output["results"]]
    return result


def burst(args, env: dict) -> dict:
    started_utc = dt.datetime.now(dt.timezone.utc).isoformat()
    barrier = threading.Barrier(args.count)
    def request(index):
        barrier.wait(timeout=30)
        return {"request": index, **search(args, env, args.queries[index])}
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.count) as pool:
        samples = list(pool.map(request, range(args.count)))
    walls = sorted(sample["wall_ms"] for sample in samples)
    p95 = walls[math.ceil(len(walls) * .95) - 1]
    successes = sum(sample["success"] for sample in samples)
    return {"count": args.count, "successes": successes, "p95_ms": p95,
            "started_utc": started_utc, "finished_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
            "p95_method": "nearest rank over every request, including failures/timeouts",
            "threshold_ms": args.threshold_ms,
            "pass": successes == args.count and p95 <= args.threshold_ms, "samples": samples}


def measure(args, env: dict) -> None:
    python, runner, manifest = materialize(args, env)
    versions = run([str(python), "-c", "import importlib.metadata as m,json; print(json.dumps({p:m.version(p) for p in ['chromadb','sentence-transformers','portalocker','torch']}))"], env, args.checkout, 60)
    save(args.output / f"{args.label}-dependency-versions.json", versions)
    try:
        installed = json.loads(versions["stdout"])
    except ValueError:
        installed = {}
    expected = {"chromadb": "1.5.5", "sentence-transformers": "5.6.0", "portalocker": "2.10.1"}
    if versions["exit_code"] != 0 or any(installed.get(package) != version for package, version in expected.items()):
        raise RuntimeError("Measured dependencies do not match the checkout requirements; no latency claim was made")
    evidence = {"label": args.label, "captured_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                "binary": digest(args.binary), "source_ref": args.source_ref,
                "runtime_runner": digest(runner), "runtime_manifest": manifest,
                "source_provenance": source_provenance(args, runner),
                "harness": digest(Path(__file__).resolve()), "dependency_versions": versions,
                "home": str(args.home), "query": QUERY, "scopes": args.scopes,
                "burst_queries": args.queries,
                "measurement_boundary": "client spawn through gwtd JSON response and process exit",
                "warm_burst_boundary": "12 fresh outer gwtd clients share the resident worker; this models a restart launch wave when the detached worker survives",
                "restart_boundary": "cold-worker recovery stress: owned resident worker terminated; new isolated coordinator root; same persisted real index",
                "limitations": ["CLI uses the GUI's Rust search implementation but does not measure browser or GUI startup.",
                                "GUI restart does not always stop detached resident workers; forced worker termination is a stronger stress condition.",
                                "Host load is observed as-is and is not fixed.",
                                "Cold phases include real model loading and are not included in the warmed phase."],
                "host_before": host_snapshot(env, args.checkout), "phases": {}, "cleanup": [], "coordinators": []}
    path = args.output / f"{args.label}-search-burst.json"
    (args.home / ".gwt/logs/perf").mkdir(parents=True, exist_ok=True)
    perf_offsets = {log.name: log.stat().st_size for log in (args.home / ".gwt/logs/perf").glob("*.jsonl")}
    try:
        evidence["cleanup"].extend(stop_owned_workers(args.home))
        evidence["coordinators"].append(reset_coordinator(args.home, f"{args.label}-cold"))
        evidence["phases"]["cold_burst"] = burst(args, env)
        save(path, evidence)
        evidence["phases"]["real_model_prewarm"] = prewarm_model(args, env, python, runner)
        evidence["worker_after_prewarm"] = worker_snapshot(args, env, evidence["phases"]["real_model_prewarm"])
        save(path, evidence)
        # Explicitly retain warmup latency and failure instead of silently
        # excluding whichever requests happened to be slow.
        evidence["phases"]["warmup"] = search(args, env)
        save(path, evidence)
        evidence["phases"]["warmed_burst"] = burst(args, env)
        evidence["worker_after_warmed_burst"] = worker_snapshot(args, env, evidence["phases"]["real_model_prewarm"])
        save(path, evidence)
        evidence["cleanup"].extend(stop_owned_workers(args.home))
        evidence["coordinators"].append(reset_coordinator(args.home, f"{args.label}-restart"))
        evidence["phases"]["restart_cold_burst"] = burst(args, env)
        save(path, evidence)
    finally:
        evidence["cleanup"].extend(stop_owned_workers(args.home))
        evidence["host_after"] = host_snapshot(env, args.checkout)
        save(path, evidence)
        logs = args.home / ".gwt/logs/perf"
        for log in logs.glob("*.jsonl"):
            with log.open("rb") as source:
                source.seek(perf_offsets.get(log.name, 0))
                (args.output / f"{args.label}-{log.name}").write_bytes(source.read())
    print(json.dumps({"path": str(path), "phases": {name: {key: phase[key] for key in
          ("count", "successes", "p95_ms", "pass") if key in phase} for name, phase in evidence["phases"].items()}}))
    if args.label != "baseline" and not evidence["phases"]["warmed_burst"]["pass"]:
        raise SystemExit(1)


def source_components(args) -> None:
    runtime = args.home / ".gwt/runtime"
    manifest = json.loads((runtime / "project_index_runtime_manifest.json").read_text(encoding="utf-8"))
    baseline = runtime / manifest["runner"]["path"]
    if baseline.read_bytes() not in args.binary.read_bytes():
        raise RuntimeError("Component baseline runner does not belong to the baseline executable")
    saved = {name: os.environ.get(name) for name in ("HOME", "USERPROFILE")}
    os.environ.update({"HOME": str(args.home), "USERPROFILE": str(args.home)})
    evidence = {"captured_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                "boundary": "Python source snapshot only; no embedding, admission, Rust transport, or browser",
                "cold_definition": "first call after a fresh module import; operating-system file caches are not cleared",
                "home": str(args.home), "baseline_binary": digest(args.binary), "samples_per_component": 3,
                "phases": ["cold", "warm_1", "warm_2"], "implementations": {}}
    fixture = args.output / "fixture.json"
    if fixture.is_file():
        fixed = json.loads(fixture.read_text(encoding="utf-8"))
        evidence["fixture_cache_tree_sha256"] = fixed["cache_tree_sha256"]
    try:
        for label, path in (("baseline", baseline),
                            ("after", args.checkout / "crates/gwt-core/runtime/chroma_index_runner.py")):
            spec = importlib.util.spec_from_file_location(f"issue_5598_component_{label}", path)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            implementation = {"runner": digest(path), "components": {"issues_source_snapshot": [], "specs_source_and_records": []}}
            for phase in evidence["phases"]:
                started = time.perf_counter()
                issues = module._issue_cache_source_snapshot(args.repo_hash)
                implementation["components"]["issues_source_snapshot"].append(
                    {"phase": phase, "wall_ms": (time.perf_counter() - started) * 1000,
                     "document_count": issues["document_count"], "fingerprint": issues["fingerprint"]})
                started = time.perf_counter()
                specs, entries = module._load_cached_spec_documents(args.repo_hash)
                records = module._build_spec_records(specs)
                implementation["components"]["specs_source_and_records"].append(
                    {"phase": phase, "wall_ms": (time.perf_counter() - started) * 1000,
                     "source_count": len(specs), "manifest_entries": len(entries), "record_count": len(records)})
            evidence["implementations"][label] = implementation
    finally:
        for name, value in saved.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
        save(args.output / "source-components.json", evidence)
    print(json.dumps({label: {name: [sample["wall_ms"] for sample in samples]
          for name, samples in implementation["components"].items()}
          for label, implementation in evidence["implementations"].items()}))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true", help="Copy canonical sources and build the fixed real index once")
    parser.add_argument("--resume-prepare", action="store_true", help="Resume preparation using the already fixed sources")
    parser.add_argument("--seed-from-active-index", action="store_true", help="Copy existing real vectors and apply native incremental indexing")
    parser.add_argument("--source-components", action="store_true", help="Measure full source snapshots without loading Chroma or the model")
    parser.add_argument("--checkout", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--binary", type=Path, required=True, help="Explicit checkout-built gwtd executable")
    parser.add_argument("--home", type=Path, required=True, help="Owned isolated HOME, reused for both measurements")
    parser.add_argument("--source-home", type=Path, default=Path.home())
    parser.add_argument("--dependency-python", type=Path, required=True)
    parser.add_argument("--model-cache", type=Path, required=True)
    parser.add_argument("--source-ref", required=True, help="Source revision from which the binary was built")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", choices=["baseline", "after"], default="baseline")
    parser.add_argument("--repo-hash", default="99a8660247f5bc49")
    parser.add_argument("--scopes", nargs="+", choices=SCOPES, default=SCOPES)
    parser.add_argument("--count", type=int, default=12)
    parser.add_argument("--query-file", type=Path, help="JSON array of count distinct nonempty burst queries")
    parser.add_argument("--timeout", type=float, default=75)
    parser.add_argument("--model-timeout", type=float, default=240)
    parser.add_argument("--build-timeout", type=float, default=1800)
    parser.add_argument("--threshold-ms", type=float, default=3000)
    args = parser.parse_args()
    args.queries = [QUERY] * args.count
    if args.query_file:
        args.queries = json.loads(args.query_file.read_text(encoding="utf-8"))
        if (not isinstance(args.queries, list) or len(args.queries) != args.count
                or any(not isinstance(query, str) or not query.strip() for query in args.queries)
                or len(set(args.queries)) != args.count):
            parser.error("--query-file must contain count distinct nonempty strings")
    for name in ("checkout", "binary", "home", "source_home", "dependency_python", "model_cache", "output"):
        setattr(args, name, getattr(args, name).resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    coordinator = args.home / ".gwt/runtime/index-coordinator"
    env = isolated_env(args.home, args.model_cache, coordinator)
    if args.source_components:
        source_components(args)
    elif args.prepare or args.resume_prepare:
        prepare(args, env)
    else:
        measure(args, env)


if __name__ == "__main__":
    main()
