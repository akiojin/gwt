#!/usr/bin/env python3
"""ChromaDB project index helper for gwt.

This helper is executed by Rust backend commands and returns JSON on stdout.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterable, Iterator, List, Optional, Sequence


def emit(payload: dict) -> None:
    # Keep stdout JSON ASCII-only so Windows locale encodings never corrupt output bytes.
    sys.stdout.write(json.dumps(payload, ensure_ascii=True))
    sys.stdout.flush()


# ---------------------------------------------------------------------
# Phase 70 (SPEC #1939 / Issue #3264): QoS thread caps and priority
# ---------------------------------------------------------------------

# FR-385: thread environment must be configured before torch /
# sentence-transformers are imported, so this section stays stdlib-only.
QOS_THREAD_CAPS = {"background": "2", "interactive": "4"}
QOS_THREAD_ENV_KEYS = (
    "OMP_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS",
    "NUMEXPR_NUM_THREADS",
    "VECLIB_MAXIMUM_THREADS",
)


def default_qos_for_action(action: str) -> str:
    """Index builds are background work; searches and light actions default
    to interactive so legacy callers keep their latency profile (FR-398)."""
    return "background" if action.startswith("index") else "interactive"


def configure_qos_threads(qos: str) -> None:
    """Apply the FR-385 QoS profile: embedding thread caps (background=2 /
    interactive=4), inter-op 1, tokenizer parallelism off, and a lowered
    process priority for background work. Must run before the lazy model
    import; it never imports torch itself."""
    caps = QOS_THREAD_CAPS.get(qos)
    if caps is None:
        raise ValueError(f"unknown qos profile: {qos}")
    for key in QOS_THREAD_ENV_KEYS:
        os.environ[key] = caps
    os.environ["TOKENIZERS_PARALLELISM"] = "false"
    torch = sys.modules.get("torch")
    if torch is not None:
        try:
            torch.set_num_threads(int(caps))
            torch.set_num_interop_threads(1)
        except Exception:
            # set_num_interop_threads raises once parallel work has started;
            # the env caps above still bound any new thread pools.
            pass
    if qos == "background":
        _lower_process_priority()


def _lower_process_priority() -> None:
    if os.name == "nt":  # pragma: no cover - Windows-only branch
        try:
            import ctypes

            below_normal_priority_class = 0x00004000
            handle = ctypes.windll.kernel32.GetCurrentProcess()
            ctypes.windll.kernel32.SetPriorityClass(handle, below_normal_priority_class)
        except Exception:
            pass
    else:
        try:
            os.nice(10)
        except OSError:
            pass


# ---------------------------------------------------------------------
# Phase 70: cooperative yield against the host-wide coordinator
# ---------------------------------------------------------------------

_QOS_PRIORITY_RANK = {
    "interactive-search": 0,
    "manual-rebuild": 1,
    "background": 2,
}


def _coordinator_root() -> Path:
    """Coordinator root shared with the Rust side. Resolution mirrors
    `gwt_index_root` (HOME / USERPROFILE) so both halves observe the same
    directory; `GWT_INDEX_COORDINATOR_ROOT` is the explicit override."""
    override = os.environ.get("GWT_INDEX_COORDINATOR_ROOT")
    if override:
        return Path(override)
    home = os.environ.get("HOME") or os.environ.get("USERPROFILE") or str(Path.home())
    return Path(home) / ".gwt" / "runtime" / "index-coordinator"


def _pending_higher_priority(than: str) -> bool:
    """True when a claimant with priority strictly higher than `than` is
    pending on the host-wide heavy lease (FR-389). Presence of the pending
    registration is the signal; stale files are swept by the Rust side, so
    the worst case is one unnecessary yield."""
    pending_dir = _coordinator_root() / "heavy.pending"
    try:
        entries = list(pending_dir.iterdir())
    except OSError:
        return False
    rank = _QOS_PRIORITY_RANK.get(than, 99)
    for path in entries:
        if path.suffix != ".json":
            continue
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if _QOS_PRIORITY_RANK.get(data.get("priority"), 99) < rank:
            return True
    return False


INDEX_PATH_POLICY_FILE = "index_path_policy.json"
FALLBACK_INDEX_PATH_POLICY = {
    "schema_version": 1,
    "max_file_size": 1_048_576,
    "allow_paths": [
        ".gwt/work/memory.md",
        ".gwt/work/discussions.md",
        ".gwt/work/events.jsonl",
    ],
    "deny_root_prefixes": [
        ".git",
        ".claude",
        ".codex",
        ".gemini",
        ".gwt",
        "tasks",
        "specs",
    ],
    "deny_directory_names": [
        "node_modules",
        "target",
        "dist",
        "build",
        ".next",
        ".nuxt",
        "vendor",
        ".venv",
        "venv",
        ".tox",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        ".gradle",
        ".terraform",
        "coverage",
        "htmlcov",
        ".turbo",
        ".parcel-cache",
        "__pycache__",
    ],
    "deny_file_extensions": [".snap"],
    "binary_extensions": [
        ".png",
        ".jpg",
        ".jpeg",
        ".gif",
        ".bmp",
        ".ico",
        ".svg",
        ".woff",
        ".woff2",
        ".ttf",
        ".eot",
        ".otf",
        ".pdf",
        ".zip",
        ".tar",
        ".gz",
        ".bz2",
        ".xz",
        ".7z",
        ".mp3",
        ".mp4",
        ".wav",
        ".avi",
        ".mov",
        ".mkv",
        ".dmg",
        ".msi",
        ".deb",
        ".rpm",
        ".AppImage",
        ".safetensors",
        ".bin",
        ".onnx",
        ".pt",
        ".pth",
    ],
}


def load_index_path_policy() -> Dict[str, Any]:
    """Load the shared project-index path policy bundled with the runner."""
    policy_path = Path(__file__).with_name(INDEX_PATH_POLICY_FILE)
    try:
        policy = json.loads(policy_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        policy = dict(FALLBACK_INDEX_PATH_POLICY)

    merged = dict(FALLBACK_INDEX_PATH_POLICY)
    merged.update(policy)
    return merged


INDEX_PATH_POLICY = load_index_path_policy()
BINARY_EXTENSIONS = {ext.lower() for ext in INDEX_PATH_POLICY["binary_extensions"]}
MAX_FILE_SIZE = int(INDEX_PATH_POLICY["max_file_size"])
CODE_COLLECTION = "files_code"
DOC_COLLECTION = "files_docs"
LEGACY_FILE_COLLECTION = "files"

SKIP_FILE_EXTENSIONS = {ext.lower() for ext in INDEX_PATH_POLICY["deny_file_extensions"]}
SKIP_ROOT_DIRECTORIES = set(INDEX_PATH_POLICY["deny_root_prefixes"])

DOC_FILE_EXTENSIONS = {
    ".md",
    ".mdx",
    ".rst",
    ".adoc",
    ".txt",
}

DOC_ROOT_DIRECTORIES = {
    "docs",
}


def normalize_rel_path(path: Path) -> str:
    """Normalize relative paths to forward-slash form across platforms."""
    return path.as_posix()


def _policy_allowlisted(rel_path: str, policy: Optional[Dict[str, Any]] = None) -> bool:
    policy = policy or INDEX_PATH_POLICY
    return rel_path.strip("/") in set(policy["allow_paths"])


def _policy_denies_path(rel_path: str, policy: Optional[Dict[str, Any]] = None) -> bool:
    policy = policy or INDEX_PATH_POLICY
    rel_path = rel_path.strip("/")
    if not rel_path or _policy_allowlisted(rel_path, policy):
        return False

    parts = [part for part in rel_path.split("/") if part]
    if not parts:
        return False

    if parts[0] in set(policy["deny_root_prefixes"]):
        return True

    if any(part in set(policy["deny_directory_names"]) for part in parts):
        return True

    suffix = Path(rel_path).suffix.lower()
    return suffix in {ext.lower() for ext in policy["deny_file_extensions"]}


def load_gitignore_patterns(project_root: Path) -> List[tuple[str, str]]:
    """Load root/nested .gitignore and project-local info/exclude patterns."""
    return load_project_ignore_patterns(project_root)


def load_project_ignore_patterns(project_root: Path) -> List[tuple[str, str]]:
    """Load project-local ignore patterns without consulting global git ignores."""
    policy = load_index_path_policy()
    patterns: List[tuple[str, str]] = []

    for root, dirs, files in os.walk(project_root):
        root_path = Path(root)
        rel_root = root_path.relative_to(project_root)
        base_rel = "" if str(rel_root) == "." else rel_root.as_posix()

        dirs[:] = [
            d for d in dirs
            if not _policy_denies_path(
                f"{base_rel}/{d}" if base_rel else d,
                policy,
            )
        ]

        if ".gitignore" not in files:
            continue
        gitignore = root_path / ".gitignore"
        for line in gitignore.read_text(encoding="utf-8", errors="replace").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                patterns.append((base_rel, line))

    info_exclude = _git_info_exclude_path(project_root)
    if info_exclude and info_exclude.is_file():
        for line in info_exclude.read_text(encoding="utf-8", errors="replace").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                patterns.append(("", line))

    return patterns


def _git_info_exclude_path(project_root: Path) -> Optional[Path]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(project_root), "rev-parse", "--git-path", "info/exclude"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    path_text = completed.stdout.strip()
    if not path_text:
        return None
    path = Path(path_text)
    return path if path.is_absolute() else project_root / path


def _pattern_to_regex(pattern: str, base_rel: str = "") -> Optional[re.Pattern]:
    """Convert a simplified gitignore-style pattern to a regex."""
    negated = pattern.startswith("!")
    if negated:
        return None

    pattern = pattern.rstrip("/")
    pattern = pattern.lstrip("/")
    if not pattern:
        return None

    base_rel = base_rel.strip("/")
    regex = pattern.replace(".", r"\.")
    regex = regex.replace("**", "{{GLOBSTAR}}")
    regex = regex.replace("*", "[^/]*")
    regex = regex.replace("{{GLOBSTAR}}", ".*")
    regex = regex.replace("?", "[^/]")

    if base_rel and "/" not in pattern:
        regex = f"^{re.escape(base_rel)}/(.*/)?{regex}(/.*|$)"
    elif base_rel:
        regex = f"^{re.escape(base_rel)}/{regex}(/.*|$)"
    elif "/" not in pattern:
        regex = f"(^|.*/){regex}(/.*|$)"
    else:
        regex = f"^{regex}(/.*|$)"

    try:
        return re.compile(regex)
    except re.error:
        return None


def should_ignore(rel_path: str, compiled_patterns: List[re.Pattern]) -> bool:
    """Check if a relative path should be ignored."""
    for pat in compiled_patterns:
        if pat.search(rel_path):
            return True
    return False


def is_binary_file(path: Path) -> bool:
    """Check if a file is likely binary."""
    return path.suffix.lower() in BINARY_EXTENSIONS


def classify_file_bucket(rel_path: str) -> str:
    """Classify a collected file into code/docs buckets or skip it entirely."""
    rel = Path(rel_path)
    parts = rel.parts
    suffix = rel.suffix.lower()

    if suffix in SKIP_FILE_EXTENSIONS:
        return "skip"

    if parts and parts[0] in SKIP_ROOT_DIRECTORIES:
        return "skip"

    if rel.name.lower().startswith("readme"):
        return "docs"

    if suffix in DOC_FILE_EXTENSIONS:
        return "docs"

    if parts and parts[0] in DOC_ROOT_DIRECTORIES:
        return "docs"

    return "code"


def collect_files(project_root: Path) -> List[Path]:
    """Recursively collect project files, respecting the shared path policy."""
    policy = load_index_path_policy()
    patterns = load_project_ignore_patterns(project_root)
    compiled = [
        p for p in (_pattern_to_regex(pat, base) for base, pat in patterns) if p is not None
    ]
    result = []
    for root, dirs, files in os.walk(project_root):
        root_path = Path(root)
        rel_root = root_path.relative_to(project_root)
        rel_root_str = str(rel_root) if str(rel_root) != "." else ""

        dirs[:] = [
            d for d in dirs
            if not _policy_denies_path(
                f"{rel_root_str}/{d}" if rel_root_str else d,
                policy,
            )
            and not should_ignore(f"{rel_root_str}/{d}" if rel_root_str else d, compiled)
        ]

        for fname in files:
            rel = f"{rel_root_str}/{fname}" if rel_root_str else fname
            if _policy_denies_path(rel, policy):
                continue
            if should_ignore(rel, compiled):
                continue
            fpath = root_path / fname
            if is_binary_file(fpath):
                continue
            try:
                if fpath.stat().st_size > MAX_FILE_SIZE:
                    continue
            except OSError:
                continue
            result.append(fpath)
    return result


def extract_description(file_path: Path) -> str:
    """Extract a short description from a file's content."""
    suffix = file_path.suffix.lower()
    name = file_path.name.lower()

    try:
        content = file_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return file_path.name

    lines = content.splitlines()
    if not lines:
        return file_path.name

    if suffix == ".rs":
        for line in lines[:20]:
            stripped = line.strip()
            if stripped.startswith("//!"):
                text = stripped[3:].strip()
                if text:
                    return text
        return file_path.name

    if suffix in (".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"):
        for line in lines[:20]:
            stripped = line.strip()
            if stripped.startswith("//") and not stripped.startswith("///"):
                text = stripped[2:].strip()
                if text and not text.startswith("!"):
                    return text
            if stripped.startswith("/**"):
                text = stripped[3:].rstrip("*/").strip()
                if text:
                    return text
                for next_line in lines[1:10]:
                    next_stripped = next_line.strip().lstrip("* ").rstrip("*/").strip()
                    if next_stripped:
                        return next_stripped
                break
        return file_path.name

    if suffix == ".svelte":
        in_script = False
        for line in lines[:40]:
            stripped = line.strip()
            if "<script" in stripped:
                in_script = True
                continue
            if in_script:
                if stripped.startswith("//"):
                    text = stripped[2:].strip()
                    if text:
                        return text
                elif stripped.startswith("/**"):
                    text = stripped[3:].rstrip("*/").strip()
                    if text:
                        return text
                elif stripped and not stripped.startswith("import") and not stripped.startswith("export"):
                    break
        return file_path.name

    if suffix in (".md", ".mdx"):
        for line in lines[:10]:
            stripped = line.strip()
            if stripped.startswith("# "):
                return stripped[2:].strip()
        return file_path.name

    if suffix == ".toml" or name == "cargo.toml":
        for line in lines[:50]:
            stripped = line.strip()
            if stripped.startswith("description"):
                match = re.match(r'description\s*=\s*"(.+?)"', stripped)
                if match:
                    return match.group(1)
        return file_path.name

    if suffix in (".json", ".jsonc") and "package" in name:
        try:
            data = json.loads(content)
            if isinstance(data, dict) and "description" in data:
                return str(data["description"])
        except (json.JSONDecodeError, ValueError):
            pass
        return file_path.name

    if suffix == ".py":
        for line in lines[:10]:
            stripped = line.strip()
            if stripped.startswith('"""'):
                text = stripped[3:].removesuffix('"""').strip()
                if text:
                    return text
            if stripped.startswith("'''"):
                text = stripped[3:].removesuffix("'''").strip()
                if text:
                    return text
            if stripped.startswith("#") and not stripped.startswith("#!"):
                text = stripped[1:].strip()
                if text:
                    return text
        return file_path.name

    if suffix in (".yml", ".yaml"):
        for line in lines[:10]:
            stripped = line.strip()
            if stripped.startswith("#"):
                text = stripped[1:].strip()
                if text:
                    return text
        return file_path.name

    return file_path.name


def action_probe() -> dict:
    """Check if chromadb is available."""
    if importlib.util.find_spec("chromadb") is None:
        return {"ok": False, "error": "Missing Python package: chromadb"}

    import chromadb  # type: ignore

    return {
        "ok": True,
        "chromadbVersion": chromadb.__version__,
        "pythonVersion": sys.version.split()[0],
    }


def action_index(project_root: str, db_path: str) -> dict:
    """Index all project files into ChromaDB."""
    import chromadb  # type: ignore

    root = Path(project_root).resolve()
    db = Path(db_path).resolve()
    db.mkdir(parents=True, exist_ok=True)

    start = time.monotonic()

    client = chromadb.PersistentClient(path=str(db))
    code_collection = client.get_or_create_collection(
        name=CODE_COLLECTION,
        metadata={"hnsw:space": "cosine"},
    )
    doc_collection = client.get_or_create_collection(
        name=DOC_COLLECTION,
        metadata={"hnsw:space": "cosine"},
    )

    files = collect_files(root)
    current_code_ids = set()
    current_doc_ids = set()

    batch_size = 100
    total_code_indexed = 0
    total_doc_indexed = 0

    if files:
        for i in range(0, len(files), batch_size):
            batch = files[i : i + batch_size]
            code_ids = []
            code_documents = []
            code_metadatas = []
            doc_ids = []
            doc_documents = []
            doc_metadatas = []

            for fpath in batch:
                rel = str(fpath.relative_to(root))
                bucket = classify_file_bucket(rel)
                if bucket == "skip":
                    continue
                desc = extract_description(fpath)
                try:
                    size = fpath.stat().st_size
                except OSError:
                    size = 0

                payload = {
                    "path": rel,
                    "description": desc,
                    "file_type": fpath.suffix.lstrip(".") or "unknown",
                    "size": size,
                    "bucket": bucket,
                }

                if bucket == "docs":
                    current_doc_ids.add(rel)
                    doc_ids.append(rel)
                    doc_documents.append(f"{rel}: {desc}")
                    doc_metadatas.append(payload)
                else:
                    current_code_ids.add(rel)
                    code_ids.append(rel)
                    code_documents.append(f"{rel}: {desc}")
                    code_metadatas.append(payload)

            if code_ids:
                code_collection.upsert(
                    ids=code_ids,
                    documents=code_documents,
                    metadatas=code_metadatas,
                )
                total_code_indexed += len(code_ids)

            if doc_ids:
                doc_collection.upsert(
                    ids=doc_ids,
                    documents=doc_documents,
                    metadatas=doc_metadatas,
                )
                total_doc_indexed += len(doc_ids)

    try:
        existing_code = code_collection.get()
        stale_code = [eid for eid in existing_code["ids"] if eid not in current_code_ids]
        if stale_code:
            code_collection.delete(ids=stale_code)

        existing_docs = doc_collection.get()
        stale_docs = [eid for eid in existing_docs["ids"] if eid not in current_doc_ids]
        if stale_docs:
            doc_collection.delete(ids=stale_docs)
    except Exception as exc:
        print(f"Warning: stale entry cleanup failed: {exc}", file=sys.stderr)

    try:
        client.delete_collection(LEGACY_FILE_COLLECTION)
    except Exception:
        pass

    elapsed = int((time.monotonic() - start) * 1000)
    return {
        "ok": True,
        "filesIndexed": total_code_indexed + total_doc_indexed,
        "codeFilesIndexed": total_code_indexed,
        "docFilesIndexed": total_doc_indexed,
        "durationMs": elapsed,
    }


def _load_file_collection(client, name: str):
    """Load a file collection, falling back to the legacy collection for code search."""
    try:
        return client.get_collection(name)
    except Exception:
        if name == CODE_COLLECTION:
            return client.get_collection(LEGACY_FILE_COLLECTION)
        raise


def _search_file_collection(db_path: str, query: str, n_results: int, collection_name: str, missing_message: str) -> dict:
    """Search one of the file-oriented collections."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
        return {"ok": False, "error": f"Index not found at {db}"}

    client = chromadb.PersistentClient(path=str(db))
    try:
        collection = _load_file_collection(client, collection_name)
    except Exception:
        return {"ok": False, "error": missing_message}

    count = collection.count()
    if count == 0:
        return {"ok": True, "results": []}

    actual_n = min(n_results, count)
    results = collection.query(query_texts=[query], n_results=actual_n)

    items = []
    if results and results["ids"] and results["ids"][0]:
        for idx, file_id in enumerate(results["ids"][0]):
            meta = results["metadatas"][0][idx] if results["metadatas"] else {}
            distance = results["distances"][0][idx] if results["distances"] else None
            items.append({
                "path": meta.get("path", file_id),
                "description": meta.get("description", ""),
                "distance": round(distance, 4) if distance is not None else None,
                "fileType": meta.get("file_type", ""),
                "size": meta.get("size", 0),
            })

    if not items:
        items = fallback_substring_search(collection, query, actual_n)

    return {"ok": True, "results": items}


def _finalize_normalized_origin(host: str, path: str) -> str:
    """Mirror gwt_core::repo_hash::finalize_normalized: lowercase host + path,
    trim surrounding slashes from the path, and strip a single `.git` suffix."""
    path = path.strip("/")
    if path.endswith(".git"):
        path = path[: -len(".git")]
    return f"{host.lower()}/{path.lower()}"


def normalize_origin_url(url: str) -> str:
    """Byte-for-byte port of gwt_core::repo_hash::normalize_origin_url.

    All of these produce `github.com/akiojin/gwt`:
      - https://github.com/akiojin/gwt.git
      - https://github.com/Akiojin/gwt
      - git@github.com:akiojin/gwt.git
      - ssh://git@github.com:22/akiojin/gwt.git
    """
    s = url.strip()
    while s.endswith("/"):
        s = s[:-1]

    # 1. SSH shorthand: git@host:user/repo[.git]
    if s.startswith("git@"):
        rest = s[len("git@"):]
        idx = rest.find(":")
        if idx != -1:
            return _finalize_normalized_origin(rest[:idx], rest[idx + 1:])

    # 2. scheme://[user[:pass]@]host[:port]/path
    scheme_end = s.find("://")
    if scheme_end != -1:
        after_scheme = s[scheme_end + 3:]
        at = after_scheme.find("@")
        after_user = after_scheme[at + 1:] if at != -1 else after_scheme
        slash = after_user.find("/")
        if slash != -1:
            host_port = after_user[:slash]
            host = host_port.split(":", 1)[0]
            return _finalize_normalized_origin(host, after_user[slash + 1:])

    # 3. Bare host/path form (already mostly normalized).
    slash = s.find("/")
    if slash != -1:
        return _finalize_normalized_origin(s[:slash], s[slash + 1:])

    return s.lower()


def compute_repo_hash(origin_url: str) -> str:
    """SHA256[:16] of the normalized origin URL (matches Rust RepoHash)."""
    return hashlib.sha256(
        normalize_origin_url(origin_url).encode("utf-8")
    ).hexdigest()[:16]


def compute_worktree_hash(worktree_path: str) -> str:
    """SHA256[:16] of the canonicalized worktree path (matches Rust WorktreeHash)."""
    return hashlib.sha256(
        str(Path(worktree_path).resolve()).encode("utf-8")
    ).hexdigest()[:16]


def _git_origin_url(project_root: str) -> Optional[str]:
    """Return the configured `origin` remote URL for `project_root`, or None.

    Uses `git remote get-url origin` (which applies any `url.*.insteadOf`
    rewrites), matching the Rust launch-time `detect_repo_hash_for_dir`.
    """
    try:
        result = subprocess.run(
            ["git", "remote", "get-url", "origin"],
            cwd=str(project_root),
            capture_output=True,
            encoding="utf-8",
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError, OSError):
        return None
    url = result.stdout.strip()
    return url or None


def _derive_hashes_from_project_root(project_root: str) -> Optional[Dict[str, str]]:
    """Derive (repo_hash, worktree_hash) from a worktree path.

    Used when the caller omitted --repo-hash/--worktree-hash (e.g. an agent
    pane whose launch environment did not export GWT_REPO_HASH /
    GWT_WORKTREE_HASH). The derived hashes match the Rust canonical
    implementations so they address the same on-disk index the gwt app builds.

    Returns None when project_root is empty or no `origin` remote is found.
    """
    if not project_root:
        return None
    url = _git_origin_url(project_root)
    if not url:
        return None
    try:
        worktree_hash = compute_worktree_hash(project_root)
    except OSError:
        return None
    return {
        "repo_hash": compute_repo_hash(url),
        "worktree_hash": worktree_hash,
    }


def _legacy_to_v2_args(db_path: str) -> Optional[Dict[str, str]]:
    """Derive (repo_hash, worktree_hash, project_root) from a legacy
    `--db-path = $WORKTREE/.gwt/index` argument so the legacy entrypoints
    can transparently fall through to the v2 auto-build pipeline.

    Returns None when the path does not match the legacy pattern.
    """
    p = Path(db_path).resolve()
    # Expected layout: <worktree>/.gwt/index
    if p.name != "index":
        return None
    parent = p.parent
    if parent.name != ".gwt":
        return None
    worktree = parent.parent
    if not worktree.is_dir():
        return None
    url = _git_origin_url(str(worktree))
    if not url:
        return None
    return {
        "repo_hash": compute_repo_hash(url),
        "worktree_hash": hashlib.sha256(
            str(worktree).encode("utf-8")
        ).hexdigest()[:16],
        "project_root": str(worktree),
    }


def action_search(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search implementation-focused project files (legacy entrypoint).

    Phase 8: when the legacy index path is missing, transparently fall
    through to the v2 auto-build pipeline.
    """
    legacy_db = Path(db_path).resolve()
    if not legacy_db.is_dir():
        v2 = _legacy_to_v2_args(db_path)
        if v2:
            return action_search_v2(
                action="search-files",
                repo_hash=v2["repo_hash"],
                worktree_hash=v2["worktree_hash"],
                project_root=v2["project_root"],
                query=query,
                n_results=n_results,
                no_auto_build=False,
            )
    return _search_file_collection(
        db_path,
        query,
        n_results,
        CODE_COLLECTION,
        "Collection 'files_code' not found. Run index-files first.",
    )


def action_search_docs(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search project docs (legacy entrypoint with v2 auto-build fallback)."""
    legacy_db = Path(db_path).resolve()
    if not legacy_db.is_dir():
        v2 = _legacy_to_v2_args(db_path)
        if v2:
            return action_search_v2(
                action="search-files-docs",
                repo_hash=v2["repo_hash"],
                worktree_hash=v2["worktree_hash"],
                project_root=v2["project_root"],
                query=query,
                n_results=n_results,
                no_auto_build=False,
            )
    return _search_file_collection(
        db_path,
        query,
        n_results,
        DOC_COLLECTION,
        "Collection 'files_docs' not found. Run index-files first.",
    )


def fallback_substring_search(collection, query: str, n_results: int) -> List[dict]:
    """Fallback search using case-insensitive substring matching on path/description."""
    normalized = query.strip().lower()
    if not normalized or n_results <= 0:
        return []

    try:
        snapshot = collection.get(include=["metadatas"])
    except Exception:
        return []

    ids = snapshot.get("ids") or []
    metadatas = snapshot.get("metadatas") or []
    matches = []

    for idx, file_id in enumerate(ids):
        meta = metadatas[idx] if idx < len(metadatas) and metadatas[idx] else {}
        path = str(meta.get("path", file_id))
        description = str(meta.get("description", ""))

        path_pos = path.lower().find(normalized)
        desc_pos = description.lower().find(normalized)

        positions = [pos for pos in (path_pos, desc_pos) if pos >= 0]
        if not positions:
            continue

        rank = 0 if path_pos >= 0 else 1
        best_pos = min(positions)
        matches.append((
            rank,
            best_pos,
            path,
            {
                "path": path,
                "description": description,
                "distance": None,
                "fileType": meta.get("file_type", ""),
                "size": meta.get("size", 0),
            },
        ))

    matches.sort(key=lambda item: (item[0], item[1], item[2]))
    return [item[3] for item in matches[:n_results]]


def action_index_issues(project_root: str, db_path: str) -> dict:
    """Index GitHub Issues into ChromaDB collection 'issues'."""
    import chromadb  # type: ignore

    root = Path(project_root).resolve()
    db = Path(db_path).resolve()
    db.mkdir(parents=True, exist_ok=True)

    start = time.monotonic()

    try:
        result = subprocess.run(
            [
                "gh", "issue", "list",
                "--state", "all",
                "--limit", "200",
                "--json", "number,title,body,labels,state,url",
            ],
            cwd=str(root),
            capture_output=True,
            encoding="utf-8",
            check=True,
        )
        issues = json.loads(result.stdout)
    except subprocess.CalledProcessError as exc:
        return {"ok": False, "error": f"gh issue list failed: {exc.stderr.strip()}"}
    except (json.JSONDecodeError, ValueError) as exc:
        return {"ok": False, "error": f"Failed to parse gh output: {exc}"}

    client = chromadb.PersistentClient(path=str(db))
    collection = client.get_or_create_collection(
        name="issues",
        metadata={"hnsw:space": "cosine"},
    )

    try:
        existing = collection.get()
        if existing["ids"]:
            collection.delete(ids=existing["ids"])
    except Exception:
        pass

    if not issues:
        elapsed = int((time.monotonic() - start) * 1000)
        return {"ok": True, "issuesIndexed": 0, "durationMs": elapsed}

    ids = []
    documents = []
    metadatas = []

    for issue in issues:
        number = issue.get("number", 0)
        title = issue.get("title", "")
        body = (issue.get("body") or "")[:500]
        state = issue.get("state", "")
        url = issue.get("url", "")
        labels = [lbl.get("name", "") for lbl in issue.get("labels", [])]

        ids.append(str(number))
        documents.append(f"{title}\n{body}")
        metadatas.append({
            "number": number,
            "title": title,
            "url": url,
            "state": state,
            "labels": ",".join(labels),
        })

    batch_size = 100
    for i in range(0, len(ids), batch_size):
        collection.upsert(
            ids=ids[i : i + batch_size],
            documents=documents[i : i + batch_size],
            metadatas=metadatas[i : i + batch_size],
        )

    elapsed = int((time.monotonic() - start) * 1000)
    return {"ok": True, "issuesIndexed": len(ids), "durationMs": elapsed}


def action_search_issues(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search the GitHub Issues index (legacy entrypoint with v2 fallback)."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
        v2 = _legacy_to_v2_args(db_path)
        if v2:
            return action_search_v2(
                action="search-issues",
                repo_hash=v2["repo_hash"],
                worktree_hash=None,
                project_root=v2["project_root"],
                query=query,
                n_results=n_results,
                no_auto_build=False,
            )
        return {"ok": False, "error": f"Index not found at {db}"}

    client = chromadb.PersistentClient(path=str(db))
    try:
        collection = client.get_collection("issues")
    except Exception:
        return {"ok": False, "error": "Collection 'issues' not found. Run index-issues first."}

    count = collection.count()
    if count == 0:
        return {"ok": True, "issueResults": []}

    actual_n = min(n_results, count)
    results = collection.query(query_texts=[query], n_results=actual_n)

    items = []
    if results and results["ids"] and results["ids"][0]:
        for idx, issue_id in enumerate(results["ids"][0]):
            meta = results["metadatas"][0][idx] if results["metadatas"] else {}
            distance = results["distances"][0][idx] if results["distances"] else None
            labels_raw = meta.get("labels", "")
            labels = [lb for lb in labels_raw.split(",") if lb] if labels_raw else []
            items.append({
                "number": meta.get("number", int(issue_id)),
                "title": meta.get("title", ""),
                "url": meta.get("url", ""),
                "state": meta.get("state", ""),
                "labels": labels,
                "distance": round(distance, 4) if distance is not None else None,
            })

    return {"ok": True, "issueResults": items}


def action_index_specs(project_root: str, db_path: str) -> dict:
    """Legacy entrypoint: index local SPEC directories into collection 'specs'."""
    import chromadb  # type: ignore

    root = Path(project_root).resolve()
    db = Path(db_path).resolve()
    db.mkdir(parents=True, exist_ok=True)

    start = time.monotonic()

    specs_dir = root / "specs"
    spec_dirs = sorted(specs_dir.glob("SPEC-*")) if specs_dir.is_dir() else []

    client = chromadb.PersistentClient(path=str(db))
    collection = client.get_or_create_collection(
        name="specs",
        metadata={"hnsw:space": "cosine"},
    )

    try:
        existing = collection.get()
        if existing["ids"]:
            collection.delete(ids=existing["ids"])
    except Exception:
        pass

    ids = []
    documents = []
    metadatas = []

    for spec_path in spec_dirs:
        metadata_file = spec_path / "metadata.json"
        if not metadata_file.is_file():
            continue

        try:
            meta = json.loads(metadata_file.read_text(encoding="utf-8", errors="replace"))
        except (json.JSONDecodeError, ValueError, OSError):
            continue

        spec_id = meta.get("id", "")
        title = meta.get("title", "")
        status = meta.get("status", "")
        phase = meta.get("phase", "")
        dir_name = spec_path.name

        spec_content = ""
        spec_md = spec_path / "spec.md"
        if spec_md.is_file():
            try:
                spec_content = spec_md.read_text(encoding="utf-8", errors="replace")[:500]
            except OSError:
                pass

        ids.append(f"spec-{spec_id}")
        documents.append(f"{title}\n{spec_content}")
        metadatas.append({
            "spec_id": str(spec_id),
            "title": title,
            "status": status,
            "phase": phase,
            "dir_name": dir_name,
        })

    if ids:
        batch_size = 100
        for i in range(0, len(ids), batch_size):
            collection.upsert(
                ids=ids[i : i + batch_size],
                documents=documents[i : i + batch_size],
                metadatas=metadatas[i : i + batch_size],
            )

    elapsed = int((time.monotonic() - start) * 1000)
    return {"ok": True, "specsIndexed": len(ids), "durationMs": elapsed}


def action_search_specs(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search the SPEC index (legacy entrypoint with v2 fallback)."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
        v2 = _legacy_to_v2_args(db_path)
        if v2:
            return action_search_v2(
                action="search-specs",
                repo_hash=v2["repo_hash"],
                worktree_hash=v2["worktree_hash"],
                project_root=v2["project_root"],
                query=query,
                n_results=n_results,
                no_auto_build=False,
            )
        return {"ok": False, "error": f"Index not found at {db}"}

    client = chromadb.PersistentClient(path=str(db))
    try:
        collection = client.get_collection("specs")
    except Exception:
        return {"ok": False, "error": "Collection 'specs' not found. Run index-specs first."}

    count = collection.count()
    if count == 0:
        return {"ok": True, "specResults": []}

    actual_n = min(n_results, count)
    results = collection.query(query_texts=[query], n_results=actual_n)

    items = []
    if results and results["ids"] and results["ids"][0]:
        for idx, spec_id in enumerate(results["ids"][0]):
            meta = results["metadatas"][0][idx] if results["metadatas"] else {}
            distance = results["distances"][0][idx] if results["distances"] else None
            items.append({
                "spec_id": meta.get("spec_id", spec_id),
                "title": meta.get("title", ""),
                "status": meta.get("status", ""),
                "phase": meta.get("phase", ""),
                "dir_name": meta.get("dir_name", ""),
                "distance": round(distance, 4) if distance is not None else None,
            })

    return {"ok": True, "specResults": items}


def action_status(db_path: str) -> dict:
    """Get index status."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
        return {"ok": True, "indexed": False, "totalFiles": 0}

    client = chromadb.PersistentClient(path=str(db))
    total_code = 0
    total_docs = 0
    indexed = False

    try:
        total_code = _load_file_collection(client, CODE_COLLECTION).count()
        indexed = indexed or total_code > 0
    except Exception:
        total_code = 0

    try:
        total_docs = client.get_collection(DOC_COLLECTION).count()
        indexed = indexed or total_docs > 0
    except Exception:
        total_docs = 0

    if not indexed:
        return {"ok": True, "indexed": False, "totalFiles": 0, "totalCodeFiles": 0, "totalDocFiles": 0}

    db_size = sum(f.stat().st_size for f in db.rglob("*") if f.is_file())

    return {
        "ok": True,
        "indexed": True,
        "totalFiles": total_code + total_docs,
        "totalCodeFiles": total_code,
        "totalDocFiles": total_docs,
        "dbSizeBytes": db_size,
    }


# =====================================================================
# Phase 8: index lifecycle redesign (FR-017〜FR-029)
# =====================================================================

INDEX_SCHEMA_VERSION = 1
ISSUE_TTL_MINUTES_DEFAULT = 15
MANIFEST_FILENAME = "manifest.json"
LOCK_FILENAME = ".lock"
META_FILENAME = "meta.json"

V2_SCOPES = ("issues", "specs", "memory", "discussions", "board", "works", "files", "files-docs")
WORKTREE_SCOPED = {"files", "files-docs"}

V2_FILES_CODE_COLLECTION = "files_code"
V2_FILES_DOCS_COLLECTION = "files_docs"
V2_SPECS_COLLECTION = "specs"
V2_ISSUES_COLLECTION = "issues"
V2_MEMORY_COLLECTION = "memory"
V2_DISCUSSIONS_COLLECTION = "discussions"
V2_BOARD_COLLECTION = "board"
V2_WORKS_COLLECTION = "works"


def gwt_index_root() -> Path:
    """Return the root directory for all gwt vector index data."""
    home = Path(os.environ.get("HOME") or os.environ.get("USERPROFILE") or Path.home())
    return home / ".gwt" / "index"


def resolve_db_path(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
) -> Path:
    """Compute the on-disk DB directory for the given (repo, worktree, scope)."""
    if scope not in V2_SCOPES:
        raise ValueError(f"unknown scope: {scope}")
    if scope in WORKTREE_SCOPED and not worktree_hash:
        raise ValueError(f"scope {scope} requires worktree_hash")

    root = (db_root or gwt_index_root()).resolve()
    repo_dir = root / repo_hash

    if scope in {"issues", "specs", "memory", "discussions", "board", "works"}:
        return repo_dir / scope

    return repo_dir / "worktrees" / worktree_hash / scope


# ---------------------------------------------------------------------
# flock helpers
# ---------------------------------------------------------------------


@contextlib.contextmanager
def acquire_lock(db_path: Path, exclusive: bool = True) -> Iterator[None]:
    """Cross-process file lock around a DB directory.

    Uses portalocker when available; falls back to fcntl on POSIX and
    msvcrt on Windows. The sentinel file lives at ``<db_path>/.lock``.
    """
    db_path = Path(db_path)
    db_path.mkdir(parents=True, exist_ok=True)
    lock_path = db_path / LOCK_FILENAME

    try:
        import portalocker  # type: ignore

        flag = portalocker.LOCK_EX if exclusive else portalocker.LOCK_SH
        lock = portalocker.Lock(str(lock_path), mode="a+", flags=flag)
        fh = lock.acquire()
        try:
            yield
        finally:
            try:
                fh.flush()
            except Exception:
                pass
            try:
                lock.release()
            except Exception:
                pass
        return
    except ImportError:
        pass

    # Fallback path: fcntl on POSIX, msvcrt on Windows.
    if os.name == "nt":
        import msvcrt  # type: ignore

        fh = open(lock_path, "a+b")
        try:
            mode = msvcrt.LK_LOCK  # always blocking; Windows lacks shared locks here
            msvcrt.locking(fh.fileno(), mode, 1)
            try:
                yield
            finally:
                try:
                    fh.seek(0)
                    msvcrt.locking(fh.fileno(), msvcrt.LK_UNLCK, 1)
                except Exception:
                    pass
        finally:
            fh.close()
        return

    import fcntl  # type: ignore

    fh = open(lock_path, "a+b")
    try:
        fcntl.flock(fh, fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH)
        try:
            yield
        finally:
            try:
                fcntl.flock(fh, fcntl.LOCK_UN)
            except Exception:
                pass
    finally:
        fh.close()


# ---------------------------------------------------------------------
# Embedding model + E5 prefix handling
# ---------------------------------------------------------------------


class _FakeEmbeddingModel:
    """Deterministic hash-based embedding used by tests.

    Activated by setting GWT_INDEX_FAKE_EMBEDDING=1. Produces 32-dim
    pseudo-vectors derived from a SHA256 hash of the input text. This
    avoids downloading the real e5 model in the unit-test suite.
    """

    DIM = 32

    def encode(self, texts: Sequence[str], **_: Any) -> List[List[float]]:
        out: List[List[float]] = []
        for text in texts:
            digest = hashlib.sha256(text.encode("utf-8")).digest()
            vec = [(digest[i] / 255.0) - 0.5 for i in range(self.DIM)]
            out.append(vec)
        return out


_MODEL_CACHE: Optional[Any] = None


def _get_embedding_model() -> Any:
    """Lazily load (and cache) the embedding model.

    Honors GWT_INDEX_FAKE_EMBEDDING=1 to substitute a deterministic
    hash-based fake. Otherwise loads ``intfloat/multilingual-e5-base``.
    """
    global _MODEL_CACHE
    if _MODEL_CACHE is not None:
        return _MODEL_CACHE

    if os.environ.get("GWT_INDEX_FAKE_EMBEDDING") == "1":
        _MODEL_CACHE = _FakeEmbeddingModel()
        return _MODEL_CACHE

    from sentence_transformers import SentenceTransformer  # type: ignore

    _MODEL_CACHE = SentenceTransformer("intfloat/multilingual-e5-base")
    return _MODEL_CACHE


class E5EmbeddingFunction:
    """Custom Chroma EmbeddingFunction that prepends e5 prefixes.

    e5 family models require ``passage: `` for documents and ``query: ``
    for queries. Existing prefixes are detected to avoid double-application.

    Compatible with both ChromaDB's plural-`input` protocol and a
    convenience single-string call for `embed_query`.
    """

    def __init__(self, model: Optional[Any] = None) -> None:
        self._model = model

    def _model_or_default(self) -> Any:
        return self._model if self._model is not None else _get_embedding_model()

    @staticmethod
    def _prefix(items: Sequence[str], tag: str) -> List[str]:
        prepared: List[str] = []
        for text in items:
            if text.startswith(f"{tag}: "):
                prepared.append(text)
            else:
                prepared.append(f"{tag}: {text}")
        return prepared

    def _to_list(self, input_value: Any) -> List[str]:
        if isinstance(input_value, str):
            return [input_value]
        return list(input_value)

    def embed_documents(self, input: Any = None, **kwargs: Any) -> List[List[float]]:  # noqa: A002
        if input is None:
            input = kwargs.get("input")  # noqa: A001
        prepared = self._prefix(self._to_list(input), "passage")
        out = self._model_or_default().encode(prepared)
        return [list(v) for v in out]

    def embed_query(self, input: Any = None, **kwargs: Any) -> List[List[float]]:  # noqa: A002
        if input is None:
            input = kwargs.get("input")  # noqa: A001
        prepared = self._prefix(self._to_list(input), "query")
        out = self._model_or_default().encode(prepared)
        return [list(v) for v in out]

    # Chroma EmbeddingFunction protocol: callable on a sequence of strings.
    # Default to passage mode (used during indexing).
    def __call__(self, input: Sequence[str]) -> List[List[float]]:  # noqa: A002
        return self.embed_documents(input)

    # Chroma >= 0.4 expects this attribute on EmbeddingFunctions for telemetry.
    def name(self) -> str:  # pragma: no cover - trivial
        return "e5-multilingual-base"

    # Newer chromadb checks `is_legacy()` to silence the legacy-config warning.
    @staticmethod
    def is_legacy() -> bool:  # pragma: no cover - trivial
        return False


def _make_chroma_collection(db_path: Path, collection_name: str):
    """Create or open a chroma collection wired with the e5 embedding fn."""
    import chromadb  # type: ignore

    db_path.mkdir(parents=True, exist_ok=True)
    client = chromadb.PersistentClient(path=str(db_path))
    ef = E5EmbeddingFunction()
    return client, client.get_or_create_collection(
        name=collection_name,
        embedding_function=ef,
        metadata={"hnsw:space": "cosine"},
    )


def _make_chroma_collection_repairing(db_path: Path, collection_name: str):
    try:
        return _make_chroma_collection(db_path, collection_name)
    except Exception:
        _reset_chroma_store(db_path)
        return _make_chroma_collection(db_path, collection_name)


def _reset_chroma_store(db_path: Path) -> None:
    db_path.mkdir(parents=True, exist_ok=True)
    for child in db_path.iterdir():
        if child.name == LOCK_FILENAME:
            continue
        if child.is_dir():
            shutil.rmtree(child, ignore_errors=True)
        else:
            try:
                child.unlink()
            except OSError:
                pass


def _open_chroma_collection(db_path: Path, collection_name: str):
    """Open an existing collection without silently creating a new one."""
    import chromadb  # type: ignore

    client = chromadb.PersistentClient(path=str(db_path))
    ef = E5EmbeddingFunction()
    try:
        collection = client.get_collection(
            name=collection_name,
            embedding_function=ef,
        )
    except Exception:
        _close_chroma_client(client)
        raise
    return client, collection


# ---------------------------------------------------------------------
# Manifest helpers (incremental indexing)
# ---------------------------------------------------------------------


def _manifest_path(worktree_dir: Path, scope: str) -> Path:
    """Manifest lives at the worktree-level (one per scope).

    The argument may be either the worktree-level dir
    (`.../worktrees/<wt>/`) or a scope-leaf dir
    (`.../worktrees/<wt>/files/`); both are normalized to the worktree
    level so writers and readers always agree on the location.
    """
    if worktree_dir.name in (
        "specs",
        "files",
        "files-docs",
        "issues",
        "memory",
        "discussions",
        "board",
        "works",
    ):
        return worktree_dir.parent / f"manifest-{scope}.json"
    return worktree_dir / f"manifest-{scope}.json"


def read_manifest(worktree_dir: Path, scope: str) -> List[Dict[str, Any]]:
    """Read the manifest for the given scope. Returns [] if missing."""
    path = _manifest_path(worktree_dir, scope)
    if not path.is_file():
        return []
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return []
    if isinstance(payload, dict) and isinstance(payload.get("entries"), list):
        return payload["entries"]
    if isinstance(payload, list):
        return payload
    return []


def write_manifest(worktree_dir: Path, scope: str, entries: List[Dict[str, Any]]) -> None:
    path = _manifest_path(worktree_dir, scope)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema_version": INDEX_SCHEMA_VERSION,
        "scope": scope,
        "entries": entries,
    }
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")


def compute_manifest_diff(
    old: List[Dict[str, Any]],
    new: List[Dict[str, Any]],
) -> Dict[str, List[str]]:
    old_map = {entry["path"]: entry for entry in old}
    new_map = {entry["path"]: entry for entry in new}
    added = sorted(set(new_map) - set(old_map))
    removed = sorted(set(old_map) - set(new_map))
    changed: List[str] = []
    for path in sorted(set(new_map) & set(old_map)):
        if (
            new_map[path].get("mtime") != old_map[path].get("mtime")
            or new_map[path].get("size") != old_map[path].get("size")
        ):
            changed.append(path)
    return {"added": added, "changed": changed, "removed": removed}


def _scan_files(project_root: Path, bucket_filter: Optional[str]) -> List[Path]:
    """Scan project files honoring the existing classify_file_bucket rules."""
    files = collect_files(project_root)
    if bucket_filter is None:
        return files
    out: List[Path] = []
    for fpath in files:
        rel = normalize_rel_path(fpath.relative_to(project_root))
        if classify_file_bucket(rel) == bucket_filter:
            out.append(fpath)
    return out


def _content_hash_for(path: Path) -> Optional[str]:
    """Stable content hash for FR-391 embedding reuse."""
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()[:16]
    except OSError:
        return None


def _build_manifest_entries(project_root: Path, paths: List[Path]) -> List[Dict[str, Any]]:
    entries: List[Dict[str, Any]] = []
    for fpath in paths:
        try:
            stat = fpath.stat()
        except OSError:
            continue
        rel = normalize_rel_path(fpath.relative_to(project_root))
        entries.append({
            "path": rel,
            "mtime": int(stat.st_mtime),
            "size": int(stat.st_size),
            "content_hash": _content_hash_for(fpath),
        })
    entries.sort(key=lambda e: e["path"])
    return entries


# ---------------------------------------------------------------------
# Stderr NDJSON progress
# ---------------------------------------------------------------------


def emit_progress(payload: dict) -> None:
    try:
        sys.stderr.write(json.dumps(payload, ensure_ascii=True) + "\n")
        sys.stderr.flush()
    except Exception:  # pragma: no cover
        pass


# ---------------------------------------------------------------------
# Document embedding helper used by full + incremental indexing
# ---------------------------------------------------------------------


def embed_documents_for_paths(
    paths: List[Path],
    project_root: Path,
    collection,
) -> int:
    """Compute embeddings for the given paths and upsert into the collection.

    Returns the number of paths actually upserted (skipping unreadable files).
    Tests patch this function with `wraps=` to count incremental re-embeds.
    """
    if not paths:
        return 0

    ids: List[str] = []
    documents: List[str] = []
    metadatas: List[Dict[str, Any]] = []

    for fpath in paths:
        try:
            rel = normalize_rel_path(fpath.relative_to(project_root))
        except ValueError:
            continue
        bucket = classify_file_bucket(rel)
        if bucket == "skip":
            continue
        try:
            stat = fpath.stat()
        except OSError:
            continue
        desc = extract_description(fpath)
        try:
            text = fpath.read_text(encoding="utf-8", errors="replace")[:2000]
        except OSError:
            text = ""
        ids.append(rel)
        documents.append(
            build_embedding_document(
                rel_path=rel,
                description=desc,
                text=text,
                bucket=bucket,
                file_type=fpath.suffix.lstrip(".") or "unknown",
            )
        )
        metadatas.append(
            {
                "path": rel,
                "description": desc,
                "file_type": fpath.suffix.lstrip(".") or "unknown",
                "size": int(stat.st_size),
                "bucket": bucket,
            }
        )

    if not ids:
        return 0

    batch = 64
    for i in range(0, len(ids), batch):
        collection.upsert(
            ids=ids[i : i + batch],
            documents=documents[i : i + batch],
            metadatas=metadatas[i : i + batch],
        )
    return len(ids)


def _delete_paths_from_collection(collection, rel_paths: Sequence[str]) -> None:
    if not rel_paths:
        return
    try:
        collection.delete(ids=list(rel_paths))
    except Exception:
        pass


def build_embedding_document(
    rel_path: str,
    description: str,
    text: str,
    bucket: str,
    file_type: str,
) -> str:
    """Build a structured embedding payload for code/docs files."""
    normalized_text = text.strip()
    parts = [
        f"path: {rel_path}",
        f"bucket: {bucket}",
        f"file_type: {file_type}",
        f"description: {description}",
        "content:",
    ]
    if normalized_text:
        parts.append(normalized_text)
    return "\n".join(parts)


# ---------------------------------------------------------------------
# v2 actions: index-files
# ---------------------------------------------------------------------


EMBED_CHECKPOINT_BATCH = 16
STAGING_DIR_SUFFIX = ".staging"
CONTINUATION_FILENAME = "continuation.json"
GENERATIONS_DIR_SUFFIX = ".gen"
ACTIVE_POINTER_FILENAME = "active.json"
GENERATION_GC_SECONDS = 24 * 3600


def _staging_dir_for(db_path: Path) -> Path:
    return db_path.parent / (db_path.name + STAGING_DIR_SUFFIX)


# ---------------------------------------------------------------------
# Phase 70 FR-390: versioned generation store with an atomic active pointer
# ---------------------------------------------------------------------


def generations_root(db_path: Path) -> Path:
    """Sibling directory holding immutable generations + the active pointer."""
    return db_path.parent / (db_path.name + GENERATIONS_DIR_SUFFIX)


def active_pointer_path(db_path: Path) -> Path:
    return generations_root(db_path) / ACTIVE_POINTER_FILENAME


def _read_active_pointer(db_path: Path) -> Optional[Dict[str, Any]]:
    pointer_path = active_pointer_path(db_path)
    try:
        payload = json.loads(pointer_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(payload, dict) or not isinstance(payload.get("generation"), str):
        return None
    return payload


def _active_pointer_corrupt(db_path: Path) -> bool:
    """True when an active pointer file exists but cannot be trusted."""
    pointer_path = active_pointer_path(db_path)
    if not pointer_path.is_file():
        return False
    pointer = _read_active_pointer(db_path)
    if pointer is None:
        return True
    return not (generations_root(db_path) / pointer["generation"]).is_dir()


def resolve_active_store(db_path: Path) -> Path:
    """Directory readers must open: the active generation when the pointer is
    valid, else the legacy scope directory (AS-17 lazy migration)."""
    pointer = _read_active_pointer(db_path)
    if pointer is not None:
        generation_dir = generations_root(db_path) / pointer["generation"]
        if generation_dir.is_dir():
            return generation_dir
    return db_path


def _publish_generation(
    db_path: Path,
    staging: Path,
    scope: str,
    document_count: int,
    source_fingerprint: Optional[str] = None,
    after_publish=None,
) -> Dict[str, Any]:
    """Atomically publish `staging` as the new active generation (FR-390).

    Under the exclusive target lock: rename the staging store into an
    immutable generation directory, atomically replace `active.json`, clean
    the legacy in-place store (lazy migration), and GC generations that have
    been abandoned for more than 24 hours (keeping the previous active one).
    Any OS failure returns a typed `PUBLISH_FAILED` payload — the previous
    active generation stays untouched.
    """
    gen_root = generations_root(db_path)
    generation_name = f"gen-{int(time.time() * 1000)}-{os.getpid()}"
    try:
        gen_root.mkdir(parents=True, exist_ok=True)
        with acquire_lock(db_path, exclusive=True):
            previous = _read_active_pointer(db_path)
            generation_dir = gen_root / generation_name
            os.replace(staging, generation_dir)
            for residue in (CONTINUATION_FILENAME, LOCK_FILENAME):
                with contextlib.suppress(OSError):
                    (generation_dir / residue).unlink()
            pointer_payload = {
                "schema_version": INDEX_SCHEMA_VERSION,
                "generation": generation_name,
                "scope": scope,
                "document_count": document_count,
                "published_at": _now_utc().isoformat(),
            }
            if source_fingerprint is not None:
                pointer_payload["source_fingerprint"] = source_fingerprint
            pointer_tmp = gen_root / f".{ACTIVE_POINTER_FILENAME}.tmp-{os.getpid()}"
            pointer_tmp.write_text(
                json.dumps(pointer_payload, ensure_ascii=True), encoding="utf-8"
            )
            os.replace(pointer_tmp, active_pointer_path(db_path))
            if after_publish is not None:
                # PR #3301 review: manifest / meta belong to the same
                # publication as the pointer swap, so run them while the
                # exclusive target lock is still held. The pointer is
                # already committed; a failure here degrades to the
                # count-mismatch repair path instead of failing the publish.
                with contextlib.suppress(Exception):
                    after_publish()
            # Lazy migration (AS-17): the legacy in-place store is no longer
            # read once a pointer exists; drop its chroma files (metadata
            # like the issues meta.json stays in place).
            _remove_legacy_chroma_files(db_path)
            _gc_abandoned_generations(
                gen_root,
                keep={
                    generation_name,
                    previous.get("generation") if previous else None,
                },
            )
    except OSError as error:
        return {
            "ok": False,
            "error_code": "PUBLISH_FAILED",
            "error": f"failed to publish index generation: {error}",
            "scope": scope,
            "retryable": True,
        }
    return {"ok": True, "generation": generation_name}


def _gc_abandoned_generations(gen_root: Path, keep: set) -> None:
    """Remove generation directories abandoned for more than 24 hours.

    The active and previous generations are always kept; recently abandoned
    directories are retained so a crashed build can be inspected / resumed.
    Deletion failures (e.g. Windows open handles) are ignored — the next
    publish retries.
    """
    now = time.time()
    try:
        entries = list(gen_root.iterdir())
    except OSError:
        return
    for entry in entries:
        if not entry.is_dir() or entry.name in keep:
            continue
        try:
            abandoned_for = now - entry.stat().st_mtime
        except OSError:  # pragma: no cover - entry vanished mid-scan
            continue
        if abandoned_for > GENERATION_GC_SECONDS:
            shutil.rmtree(entry, ignore_errors=True)


def _looks_like_chroma_segment(name: str) -> bool:
    return bool(re.fullmatch(r"[0-9a-fA-F-]{36}", name))


def _remove_legacy_chroma_files(db_path: Path) -> None:
    """Drop chroma store artifacts from a legacy scope dir after a
    generation publish, preserving metadata files (e.g. issues meta.json)."""
    with contextlib.suppress(OSError):
        (db_path / "chroma.sqlite3").unlink()
    try:
        children = list(db_path.iterdir())
    except OSError:
        return
    for child in children:
        if child.is_dir() and _looks_like_chroma_segment(child.name):
            shutil.rmtree(child, ignore_errors=True)


def _full_build_store(db_path: Path, mode: str) -> tuple:
    """Destination store for an index build (FR-390).

    Incremental updates write into the resolved active store; full rebuilds
    write into a fresh staging dir that is atomically published afterwards —
    the live store is never reset in place.
    """
    if mode == "incremental":
        return resolve_active_store(db_path), None
    staging = _staging_dir_for(db_path)
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True, exist_ok=True)
    return staging, staging


def _finish_full_build(db_path: Path, staging: Optional[Path], scope: str):
    """Publish a staged full build; no-op for incremental builds.

    Returns the typed error payload when publishing failed, else None.
    """
    if staging is None:
        return None
    try:
        client, collection = _open_chroma_collection(
            staging, _scope_collection_name(scope)
        )
        try:
            document_count = _safe_collection_count(collection)
        finally:
            _close_chroma_client(client)
    except Exception as error:
        # PR #3301 review (Critical): an unverifiable staging build must
        # never replace the healthy active generation.
        return {
            "ok": False,
            "error_code": "BUILD_VERIFY_FAILED",
            "error": f"failed to verify staging generation: {error}",
            "scope": scope,
            "retryable": True,
        }
    publish = _publish_generation(
        db_path, staging, scope=scope, document_count=document_count
    )
    return None if publish.get("ok") else publish


def _manifest_fingerprint(entries: List[Dict[str, Any]]) -> str:
    payload = json.dumps(entries, sort_keys=True, ensure_ascii=True).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()[:16]


def _write_continuation(
    continuation_path: Path,
    scope: str,
    fingerprint: str,
    done: int,
    total: int,
    reused: int = 0,
) -> None:
    continuation_path.parent.mkdir(parents=True, exist_ok=True)
    continuation_path.write_text(
        json.dumps(
            {
                "schema_version": INDEX_SCHEMA_VERSION,
                "scope": scope,
                "fingerprint": fingerprint,
                "done": done,
                "total": total,
                "reused": reused,
            },
            ensure_ascii=True,
        ),
        encoding="utf-8",
    )


def _read_continuation(continuation_path: Path) -> Optional[Dict[str, Any]]:
    try:
        return json.loads(continuation_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def action_index_files_v2(
    project_root: str,
    repo_hash: str,
    worktree_hash: str,
    mode: str = "full",
    db_root: Optional[Path] = None,
    scope: str = "files",
    qos: str = "background",
) -> dict:
    """Index project files into ChromaDB under the v2 layout.

    Full mode builds into a resumable staging store and only replaces the
    active store after the staging build finished (Phase 70 FR-389/FR-390):
    the previous index keeps serving reads during the build, and a
    background build yields at 16-document checkpoints when a
    higher-priority claimant is pending on the heavy lease.
    """
    root = Path(project_root).resolve()

    db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
    bucket = "code" if scope == "files" else "docs"

    paths = _scan_files(root, bucket_filter=bucket)
    new_entries = _build_manifest_entries(root, paths)

    emit_progress(
        {
            "phase": "indexing",
            "scope": scope,
            "mode": mode,
            "done": 0,
            "total": len(new_entries),
        }
    )

    if mode != "incremental":
        return _index_files_full_with_staging(
            root=root,
            db_path=db_path,
            scope=scope,
            paths=paths,
            new_entries=new_entries,
            repo_hash=repo_hash,
            worktree_hash=worktree_hash,
            db_root=db_root,
            qos=qos,
        )

    with acquire_lock(db_path, exclusive=True):
        # PR #3301 review: after full mode published a generation, readers
        # resolve through the active pointer — incremental updates must land
        # in that same store, never in the migrated legacy path.
        store = resolve_active_store(db_path)
        client, collection = _make_chroma_collection(
            store,
            V2_FILES_CODE_COLLECTION if scope == "files" else V2_FILES_DOCS_COLLECTION,
        )
        try:
            old_entries = read_manifest(db_path, scope=scope)
            diff = compute_manifest_diff(old_entries, new_entries)
            to_embed = diff["added"] + diff["changed"]
            to_delete = diff["removed"]

            embedded_paths = [root / rel for rel in to_embed]
            newly_embedded = embed_documents_for_paths(embedded_paths, root, collection)
            _delete_paths_from_collection(collection, to_delete)
            emit_progress(
                {
                    "phase": "diff",
                    "scope": scope,
                    "added": len(diff["added"]),
                    "changed": len(diff["changed"]),
                    "removed": len(diff["removed"]),
                }
            )

            write_manifest(db_path, scope=scope, entries=new_entries)
            actual_count = _safe_collection_count(collection)
            _write_scope_meta(
                repo_hash=repo_hash,
                worktree_hash=worktree_hash,
                scope=scope,
                db_root=db_root,
                updates={
                    "last_repair_at": _now_utc().isoformat(),
                    "document_count": actual_count,
                },
            )
        finally:
            _close_chroma_client(client)

    emit_progress(
        {
            "phase": "complete",
            "scope": scope,
            "mode": mode,
            "indexed": actual_count,
            "total": len(new_entries),
        }
    )
    return {
        "ok": True,
        "scope": scope,
        "indexed": actual_count,
        "total": len(new_entries),
        "newly_embedded": newly_embedded,
    }


def _copy_unchanged_records(
    db_path: Path,
    scope: str,
    staging_collection,
    collection_name: str,
    new_hashes: Dict[str, Optional[str]],
) -> int:
    """FR-391: copy records whose content hash is unchanged from the previous
    verified generation into the staging store, reusing their embeddings."""
    prev_entries = read_manifest(db_path, scope=scope)
    prev_hashes = {
        entry.get("path"): entry.get("content_hash") for entry in prev_entries
    }
    unchanged = [
        rel
        for rel, content_hash in new_hashes.items()
        if content_hash and prev_hashes.get(rel) == content_hash
    ]
    if not unchanged:
        return 0
    prev_store = resolve_active_store(db_path)
    if not (prev_store / "chroma.sqlite3").exists():
        return 0
    copied = 0
    try:
        with acquire_lock(db_path, exclusive=False):
            client, previous = _open_chroma_collection(prev_store, collection_name)
            try:
                batch = 64
                for start in range(0, len(unchanged), batch):
                    ids = unchanged[start : start + batch]
                    got = previous.get(
                        ids=ids, include=["embeddings", "documents", "metadatas"]
                    )
                    got_ids = got.get("ids") or []
                    embeddings = got.get("embeddings")
                    if not got_ids or embeddings is None:  # pragma: no cover - store drift
                        continue
                    staging_collection.upsert(
                        ids=list(got_ids),
                        embeddings=[list(vector) for vector in embeddings],
                        documents=got.get("documents"),
                        metadatas=got.get("metadatas"),
                    )
                    copied += len(got_ids)
            finally:
                _close_chroma_client(client)
    except Exception:
        # Reuse is an optimization: whatever was not copied is re-embedded.
        return copied
    return copied


def _index_files_full_with_staging(
    root: Path,
    db_path: Path,
    scope: str,
    paths: List[Path],
    new_entries: List[Dict[str, Any]],
    repo_hash: str,
    worktree_hash: str,
    db_root: Optional[Path],
    qos: str,
) -> dict:
    """Full rebuild via a resumable staging store and an atomic generation
    publish (Phase 70 FR-389/FR-390/FR-391)."""
    collection_name = (
        V2_FILES_CODE_COLLECTION if scope == "files" else V2_FILES_DOCS_COLLECTION
    )
    bucket = "code" if scope == "files" else "docs"
    staging = _staging_dir_for(db_path)
    continuation_path = staging / CONTINUATION_FILENAME
    fingerprint = _manifest_fingerprint(new_entries)
    total = len(new_entries)

    continuation = _read_continuation(continuation_path)
    if continuation is not None and (
        continuation.get("scope") != scope
        or continuation.get("fingerprint") != fingerprint
    ):
        # Sources changed since the parked build: the staged embeddings can
        # no longer be trusted wholesale, restart the staging build.
        shutil.rmtree(staging, ignore_errors=True)
        continuation = None
    staging.mkdir(parents=True, exist_ok=True)

    new_hashes: Dict[str, Optional[str]] = {
        entry["path"]: entry.get("content_hash") for entry in new_entries
    }
    newly_embedded = 0
    reused = 0
    yielded = False
    with acquire_lock(staging, exclusive=True):
        client, collection = _make_chroma_collection_repairing(staging, collection_name)
        try:
            staged_ids: set = set()
            if continuation is not None:
                reused = int(continuation.get("reused", 0))
                try:
                    staged_ids = set(collection.get().get("ids") or [])
                except Exception:  # pragma: no cover - defensive chroma fallback
                    staged_ids = set()
            else:
                # FR-391: reuse unchanged embeddings from the previous
                # verified generation instead of re-encoding the corpus.
                reused = _copy_unchanged_records(
                    db_path, scope, collection, collection_name, new_hashes
                )
                if reused:
                    try:
                        staged_ids = set(collection.get().get("ids") or [])
                    except Exception:  # pragma: no cover - defensive chroma fallback
                        staged_ids = set()
            pending_paths: List[Path] = []
            for fpath in paths:
                try:
                    rel = normalize_rel_path(fpath.relative_to(root))
                except ValueError:
                    continue
                if rel not in staged_ids:
                    pending_paths.append(fpath)

            for start in range(0, len(pending_paths), EMBED_CHECKPOINT_BATCH):
                batch_paths = pending_paths[start : start + EMBED_CHECKPOINT_BATCH]
                newly_embedded += embed_documents_for_paths(batch_paths, root, collection)
                done = len(staged_ids) + newly_embedded
                _write_continuation(
                    continuation_path,
                    scope=scope,
                    fingerprint=fingerprint,
                    done=done,
                    total=total,
                    reused=reused,
                )
                emit_progress(
                    {
                        "phase": "indexing",
                        "scope": scope,
                        "mode": "full",
                        "done": done,
                        "total": total,
                    }
                )
                remaining = len(pending_paths) - (start + len(batch_paths))
                if remaining > 0 and qos == "background" and _pending_higher_priority("background"):
                    yielded = True
                    break
            staged_count = len(staged_ids) + newly_embedded
            actual_count = _safe_collection_count(collection)
        finally:
            _close_chroma_client(client)

    if yielded:
        emit_progress(
            {
                "phase": "yielded",
                "scope": scope,
                "staged": staged_count,
                "total": total,
            }
        )
        return {
            "ok": True,
            "scope": scope,
            "yielded": True,
            "resumable": True,
            "indexed": staged_count,
            "total": total,
            "newly_embedded": newly_embedded,
        }

    # FR-392 / AS-10: late revalidation — the sources may have moved while
    # the staging build ran; a stale generation must never be published.
    revalidated_entries = _build_manifest_entries(
        root, _scan_files(root, bucket_filter=bucket)
    )
    if _manifest_fingerprint(revalidated_entries) != fingerprint:
        return {
            "ok": False,
            "error_code": "SOURCE_CHANGED",
            "error": "source files changed during the index build; retry",
            "scope": scope,
            "retryable": True,
        }
    if actual_count != total:
        return {
            "ok": False,
            "error_code": "BUILD_VERIFY_FAILED",
            "error": (
                f"staging store holds {actual_count} documents, expected {total}"
            ),
            "scope": scope,
            "retryable": True,
        }

    def _commit_manifest_and_meta():
        write_manifest(db_path, scope=scope, entries=new_entries)
        _write_scope_meta(
            repo_hash=repo_hash,
            worktree_hash=worktree_hash,
            scope=scope,
            db_root=db_root,
            updates={
                "last_repair_at": _now_utc().isoformat(),
                "document_count": actual_count,
            },
        )

    publish = _publish_generation(
        db_path,
        staging,
        scope=scope,
        document_count=actual_count,
        source_fingerprint=fingerprint,
        after_publish=_commit_manifest_and_meta,
    )
    if not publish.get("ok"):
        return publish

    emit_progress(
        {
            "phase": "complete",
            "scope": scope,
            "mode": "full",
            "indexed": actual_count,
            "total": total,
        }
    )
    return {
        "ok": True,
        "scope": scope,
        "indexed": actual_count,
        "total": total,
        "newly_embedded": newly_embedded,
        "reused_embeddings": reused,
    }


# ---------------------------------------------------------------------
# v2 actions: index-specs
# ---------------------------------------------------------------------


def _chunk_spec_content(content: str, max_chunk_len: int = 1800) -> List[Dict[str, str]]:
    """Split a spec.md body into semantic chunks.

    - Split primarily by H2 headings (`## ...`) so each functional area becomes
      its own embedding unit.
    - If a section exceeds `max_chunk_len`, split further by blank lines
      (paragraphs), packing paragraphs into chunks until the limit is reached.
    - Returns a list of {heading, body} dicts.
    """
    if not content.strip():
        return []

    # Split while keeping headings at the start of each resulting section.
    sections = re.split(r"(?m)^(?=## )", content)
    chunks: List[Dict[str, str]] = []
    for section in sections:
        section = section.strip()
        if not section:
            continue
        heading_match = re.match(r"^(## .+?)(?:\n|$)", section)
        heading = heading_match.group(1) if heading_match else "(intro)"
        if len(section) <= max_chunk_len:
            chunks.append({"heading": heading, "body": section})
            continue
        # Too large: pack paragraphs into smaller chunks under the cap.
        paragraphs = re.split(r"\n\s*\n", section)
        current = ""
        part = 1
        for paragraph in paragraphs:
            if not paragraph.strip():
                continue
            candidate = f"{current}\n\n{paragraph}" if current else paragraph
            if len(candidate) > max_chunk_len and current:
                chunks.append(
                    {"heading": f"{heading} [{part}]", "body": current.strip()}
                )
                part += 1
                current = paragraph
            else:
                current = candidate
        if current.strip():
            chunks.append({"heading": f"{heading} [{part}]", "body": current.strip()})
    return chunks


def action_index_specs_v2(
    project_root: str,
    repo_hash: str,
    worktree_hash: Optional[str],
    mode: str = "full",
    db_root: Optional[Path] = None,
) -> dict:
    """Index cached `gwt-spec` Issues into ChromaDB under the v2 layout."""
    db_path = resolve_db_path(repo_hash, worktree_hash, "specs", db_root=db_root)
    spec_documents, new_entries = _load_cached_spec_documents(repo_hash)
    new_entries.sort(key=lambda entry: entry["path"])

    emit_progress(
        {
            "phase": "indexing",
            "scope": "specs",
            "mode": mode,
            "done": 0,
            "total": len(spec_documents),
        }
    )

    build_store, staging = _full_build_store(db_path, mode)
    with acquire_lock(staging if staging is not None else db_path, exclusive=True):
        make_collection = (
            _make_chroma_collection
            if mode == "incremental"
            else _make_chroma_collection_repairing
        )
        client, collection = make_collection(build_store, V2_SPECS_COLLECTION)
        try:
            if mode == "incremental":
                old_entries = read_manifest(db_path, scope="specs")
                diff = compute_manifest_diff(old_entries, new_entries)
                changed_spec_ids = set(diff["added"] + diff["changed"])
                _delete_spec_records(collection, diff["changed"] + diff["removed"])
                spec_records = _build_spec_records(
                    [
                        spec
                        for spec in spec_documents
                        if spec["spec_id"] in changed_spec_ids
                    ]
                )
                emit_progress(
                    {
                        "phase": "diff",
                        "scope": "specs",
                        "added": len(diff["added"]),
                        "changed": len(diff["changed"]),
                        "removed": len(diff["removed"]),
                    }
                )
            else:
                try:
                    existing = collection.get()
                    if existing.get("ids"):
                        collection.delete(ids=existing["ids"])
                except Exception:
                    pass
                spec_records = _build_spec_records(spec_documents)

            if spec_records:
                ids = [r["id"] for r in spec_records]
                documents = [r["document"] for r in spec_records]
                metadatas = [r["metadata"] for r in spec_records]
                batch = 100
                for i in range(0, len(ids), batch):
                    collection.upsert(
                        ids=ids[i : i + batch],
                        documents=documents[i : i + batch],
                        metadatas=metadatas[i : i + batch],
                    )

            write_manifest(db_path, scope="specs", entries=new_entries)
            _write_scope_meta(
                repo_hash=repo_hash,
                worktree_hash=worktree_hash,
                scope="specs",
                db_root=db_root,
                updates={
                    "last_repair_at": _now_utc().isoformat(),
                    "document_count": len(spec_records),
                },
            )
        finally:
            _close_chroma_client(client)

    publish_error = _finish_full_build(db_path, staging, scope="specs")
    if publish_error is not None:
        return publish_error

    emit_progress(
        {
            "phase": "complete",
            "scope": "specs",
            "mode": mode,
            "indexed": len(spec_records),
            "total": len(spec_records),
        }
    )
    return {"ok": True, "scope": "specs", "indexed": len(spec_records)}


# ---------------------------------------------------------------------
# v2 actions: index-issues with TTL
# ---------------------------------------------------------------------


def _read_issue_meta(db_path: Path) -> Optional[Dict[str, Any]]:
    meta_file = db_path / META_FILENAME
    if not meta_file.is_file():
        return None
    try:
        return json.loads(meta_file.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None


def _write_issue_meta(db_path: Path, payload: Dict[str, Any]) -> None:
    db_path.mkdir(parents=True, exist_ok=True)
    (db_path / META_FILENAME).write_text(
        json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
    )


def _now_utc() -> datetime.datetime:
    return datetime.datetime.now(datetime.timezone.utc)


def _parse_iso(value: str) -> Optional[datetime.datetime]:
    try:
        return datetime.datetime.fromisoformat(value)
    except ValueError:
        return None


def _issue_cache_root(repo_hash: str) -> Path:
    home = Path(os.environ.get("HOME") or os.environ.get("USERPROFILE") or Path.home())
    return home / ".gwt" / "cache" / "issues" / repo_hash


def _issue_cache_is_populated(repo_hash: str) -> bool:
    """Return True when the GitHub Issue cache holds at least one cached issue.

    Both the issues and specs scopes build their search corpus from
    ``~/.gwt/cache/issues/<repo_hash>/`` (one-directional GitHub -> cache -> UI
    flow, SPEC-12). An absent or entry-less cache root means the cache was never
    synced for this repo-hash (or the repo-hash does not match the populated
    cache), which is distinct from a repository that genuinely has no Issues.
    """
    root = _issue_cache_root(repo_hash)
    if not root.is_dir():
        return False
    try:
        entries = root.iterdir()
    except OSError:
        return False
    for entry in entries:
        if entry.is_dir() and entry.name.isdigit() and (entry / "meta.json").is_file():
            return True
    return False


def _normalize_labels(labels: Any) -> List[str]:
    if isinstance(labels, str):
        return [labels]
    if isinstance(labels, list):
        return [label for label in labels if isinstance(label, str)]
    return []


def _phase_label(labels: Sequence[str]) -> str:
    for label in labels:
        if label.startswith("phase/"):
            return label
    return ""


def _build_cache_manifest_entry(name: str, paths: Sequence[Path]) -> Optional[Dict[str, Any]]:
    mtimes: List[int] = []
    total_size = 0
    for path in paths:
        try:
            stat = path.stat()
        except OSError:
            continue
        mtimes.append(int(stat.st_mtime))
        total_size += int(stat.st_size)
    if not mtimes:
        return None
    return {
        "path": name,
        "mtime": max(mtimes),
        "size": total_size,
    }


def _load_cached_issue_documents(repo_hash: str) -> List[Dict[str, Any]]:
    root = _issue_cache_root(repo_hash)
    if not root.is_dir():
        return []

    issues: List[Dict[str, Any]] = []
    for entry in sorted(root.iterdir(), key=lambda item: item.name):
        if not entry.is_dir():
            continue
        try:
            number = int(entry.name)
        except ValueError:
            continue
        meta_path = entry / "meta.json"
        body_path = entry / "body.md"
        if not meta_path.is_file():
            continue
        try:
            meta = json.loads(meta_path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError, ValueError, UnicodeDecodeError):
            continue
        try:
            body = (
                body_path.read_text(encoding="utf-8") if body_path.is_file() else ""
            )
        except (OSError, UnicodeDecodeError):
            body = ""
        labels = meta.get("labels", [])
        if isinstance(labels, str):
            labels = [labels]
        issues.append(
            {
                "number": number,
                "title": meta.get("title", ""),
                "body": body[:2000],
                "state": meta.get("state", ""),
                "labels": [label for label in labels if isinstance(label, str)],
            }
        )
    return issues


def _issue_cache_refresh_meta(repo_hash: str) -> Dict[str, Any]:
    meta_path = _issue_cache_root(repo_hash) / "refresh-meta.json"
    if not meta_path.is_file():
        return {}
    try:
        return json.loads(meta_path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError, UnicodeDecodeError):
        return {}


def _issue_source_fingerprint(issues: Sequence[Dict[str, Any]]) -> str:
    payload: List[Dict[str, Any]] = []
    for issue in sorted(issues, key=lambda item: int(item.get("number", 0) or 0)):
        labels = issue.get("labels", [])
        if isinstance(labels, str):
            labels = [labels]
        payload.append(
            {
                "number": int(issue.get("number", 0) or 0),
                "title": str(issue.get("title", "") or ""),
                "body": str(issue.get("body", "") or ""),
                "state": str(issue.get("state", "") or ""),
                "labels": sorted(label for label in labels if isinstance(label, str)),
            }
        )
    raw = json.dumps(
        payload,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def _issue_cache_source_snapshot(repo_hash: str) -> Dict[str, Any]:
    issues = _load_cached_issue_documents(repo_hash)
    refresh_meta = _issue_cache_refresh_meta(repo_hash)
    return {
        "fingerprint": _issue_source_fingerprint(issues),
        "document_count": len(issues),
        "cache_refresh_at": refresh_meta.get("last_full_refresh"),
    }


def _load_cached_spec_documents(
    repo_hash: str,
) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    root = _issue_cache_root(repo_hash)
    if not root.is_dir():
        return [], []

    specs: List[Dict[str, Any]] = []
    manifest_entries: List[Dict[str, Any]] = []
    for entry in sorted(root.iterdir(), key=lambda item: item.name):
        if not entry.is_dir():
            continue
        try:
            number = int(entry.name)
        except ValueError:
            continue
        meta_path = entry / "meta.json"
        if not meta_path.is_file():
            continue
        try:
            meta = json.loads(meta_path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError, ValueError, UnicodeDecodeError):
            continue

        labels = _normalize_labels(meta.get("labels", []))
        if "gwt-spec" not in labels:
            continue

        spec_path = entry / "sections" / "spec.md"
        body_path = entry / "body.md"
        source_path = spec_path if spec_path.is_file() else body_path
        try:
            content = (
                source_path.read_text(encoding="utf-8")
                if source_path.is_file()
                else ""
            )
        except (OSError, UnicodeDecodeError):
            content = ""

        manifest_entry = _build_cache_manifest_entry(
            str(number),
            [meta_path, source_path],
        )
        if manifest_entry is not None:
            manifest_entries.append(manifest_entry)

        specs.append(
            {
                "spec_id": str(meta.get("number", number)),
                "title": meta.get("title", ""),
                "status": meta.get("state", ""),
                "phase": _phase_label(labels),
                "dir_name": f"#{number}",
                "content": content,
            }
        )

    return specs, manifest_entries


_MEMORY_DATE_HEADING_RE = re.compile(
    r"^##\s+(?P<date>\d{4}-\d{2}-\d{2})\s+(?:—|--|-)\s+(?P<title>.+?)\s*$"
)
_MEMORY_BARE_HEADING_RE = re.compile(r"^##\s+(?P<title>.+?)\s*$")
_MEMORY_HEADING_CHUNK_SUFFIX_RE = re.compile(r"\s+\[\d+\]\s*$")


def _parse_memory_heading(heading: str) -> tuple[str, str]:
    """Extract (date, title) from an H2 heading.

    Handles three shapes:
    - ``## 2026-05-20 — title`` → ("2026-05-20", "title")
    - ``## title without date`` → ("", "title without date")
    - ``## 2026-05-20 — title [2]`` (paragraph-split suffix) → strip suffix
    """
    cleaned = _MEMORY_HEADING_CHUNK_SUFFIX_RE.sub("", heading)
    dated = _MEMORY_DATE_HEADING_RE.match(cleaned)
    if dated:
        return dated.group("date"), dated.group("title").strip()
    bare = _MEMORY_BARE_HEADING_RE.match(cleaned)
    if bare:
        return "", bare.group("title").strip()
    return "", heading.strip()


def _work_notes_source_path(project_root: str, repo_hash: str, file_name: str) -> Path:
    """Resolve a work-notes source file with home-first fallback.

    SPEC-3214 (FR-007): memory/discussions are machine-local scratch under
    ``~/.gwt/projects/<repo-hash>/work-notes/``. The git-tracked repo-local
    ``.gwt/work/`` file remains a read fallback for pre-migration repos.
    """
    home = Path(os.environ.get("HOME") or os.environ.get("USERPROFILE") or Path.home())
    if repo_hash:
        home_path = home / ".gwt" / "projects" / repo_hash / "work-notes" / file_name
        if home_path.is_file():
            return home_path
    return Path(project_root) / ".gwt" / "work" / file_name


def _load_memory_documents(
    project_root: str,
    repo_hash: str = "",
) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Load the project memory log and chunk it into memory units.

    The source is the machine-local home work-notes file when present,
    otherwise the legacy repo-local ``.gwt/work/memory.md`` (SPEC-3214).
    Returns a tuple ``(memory, manifest_entries)`` where ``manifest_entries``
    contains at most one entry describing the source file's mtime/size.
    Missing file → both empty. Empty file → empty memory but a manifest
    entry so that the runner can still detect future content additions.
    """
    source_path = _work_notes_source_path(project_root, repo_hash, "memory.md")
    if not source_path.is_file():
        return [], []

    try:
        content = source_path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return [], []

    try:
        stat = source_path.stat()
    except OSError:
        return [], []
    manifest_entries = [
        {
            # The manifest key switches to the absolute home path after the
            # SPEC-3214 migration so the source change triggers one re-index.
            "path": str(source_path),
            "mtime": int(stat.st_mtime),
            "size": int(stat.st_size),
        }
    ]

    chunks = _chunk_spec_content(content)
    if not chunks:
        return [], manifest_entries

    memories: List[Dict[str, Any]] = []
    grouped: Dict[tuple[str, str], int] = {}
    for chunk in chunks:
        heading = chunk["heading"]
        body = chunk["body"]
        # `_chunk_spec_content` emits a synthetic "(intro)" chunk for any
        # leading content before the first H2 (e.g. the `# Project Memory`
        # title line). That preamble is not a real memory — skip it.
        if not heading.startswith("## "):
            continue
        date, title = _parse_memory_heading(heading)
        key = (date, title)
        chunk_idx = grouped.get(key, 0)
        grouped[key] = chunk_idx + 1
        digest_input = f"{heading}\n{body}".encode("utf-8")
        memory_id = hashlib.sha1(digest_input).hexdigest()[:12]
        memories.append(
            {
                "memory_id": memory_id,
                "date": date,
                "title": title,
                "heading": heading,
                "body": body,
                "chunk_idx": chunk_idx,
                # total_chunks is filled in once the whole file is scanned
                "total_chunks": 0,
            }
        )

    for entry in memories:
        key = (entry["date"], entry["title"])
        entry["total_chunks"] = grouped[key]

    return memories, manifest_entries


def _build_memory_records(
    memories: Sequence[Dict[str, Any]],
) -> List[Dict[str, Any]]:
    """Materialize Chroma upsert records for the memory scope."""
    records: List[Dict[str, Any]] = []
    for entry in memories:
        title = entry.get("title", "")
        heading = entry.get("heading", "")
        body = entry.get("body", "")
        document = f"{title}\n{heading}\n{body}".strip()
        records.append(
            {
                "id": f"memory-{entry.get('memory_id', '')}",
                "document": document,
                "metadata": {
                    "memory_id": entry.get("memory_id", ""),
                    "date": entry.get("date", ""),
                    "title": title,
                    "heading": heading,
                    "chunk_idx": int(entry.get("chunk_idx", 0)),
                    "total_chunks": int(entry.get("total_chunks", 1)),
                },
            }
        )
    return records


def action_index_memory_v2(
    project_root: str,
    repo_hash: str,
    worktree_hash: Optional[str],
    mode: str = "full",
    db_root: Optional[Path] = None,
) -> dict:
    """Index ``.gwt/work/memory.md`` into the repo-scoped memory Chroma store.

    `worktree_hash` is accepted for symmetry with the other v2 actions but is
    ignored — memory is repo-scoped. Manifest diff degenerates to a single
    entry; when the file changes (mtime or size), all chunks are re-upserted
    after deleting prior records for the file.
    """
    del worktree_hash  # repo-scoped scope does not consume the worktree hash
    db_path = resolve_db_path(repo_hash, None, "memory", db_root=db_root)
    memories, new_entries = _load_memory_documents(project_root, repo_hash)

    emit_progress(
        {
            "phase": "indexing",
            "scope": "memory",
            "mode": mode,
            "done": 0,
            "total": len(memories),
        }
    )

    indexed = 0
    build_store, staging = _full_build_store(db_path, mode)
    with acquire_lock(staging if staging is not None else db_path, exclusive=True):
        make_collection = (
            _make_chroma_collection
            if mode == "incremental"
            else _make_chroma_collection_repairing
        )
        client, collection = make_collection(build_store, V2_MEMORY_COLLECTION)
        try:
            old_entries = read_manifest(db_path, scope="memory")
            diff = compute_manifest_diff(old_entries, new_entries)
            file_changed = bool(diff["added"] or diff["changed"] or diff["removed"])

            if mode != "incremental" or file_changed:
                try:
                    existing = collection.get()
                    if existing.get("ids"):
                        collection.delete(ids=existing["ids"])
                except Exception:
                    pass
                memory_records = _build_memory_records(memories)
            else:
                memory_records = []

            emit_progress(
                {
                    "phase": "diff",
                    "scope": "memory",
                    "added": len(diff["added"]),
                    "changed": len(diff["changed"]),
                    "removed": len(diff["removed"]),
                }
            )

            if memory_records:
                ids = [r["id"] for r in memory_records]
                documents = [r["document"] for r in memory_records]
                metadatas = [r["metadata"] for r in memory_records]
                batch = 100
                for i in range(0, len(ids), batch):
                    collection.upsert(
                        ids=ids[i : i + batch],
                        documents=documents[i : i + batch],
                        metadatas=metadatas[i : i + batch],
                    )
                indexed = len(memory_records)

            write_manifest(db_path, scope="memory", entries=new_entries)
            _write_scope_meta(
                repo_hash=repo_hash,
                worktree_hash=None,
                scope="memory",
                db_root=db_root,
                updates={
                    "last_repair_at": _now_utc().isoformat(),
                    "document_count": indexed,
                },
            )
        finally:
            _close_chroma_client(client)

    publish_error = _finish_full_build(db_path, staging, scope="memory")
    if publish_error is not None:
        return publish_error

    emit_progress(
        {
            "phase": "complete",
            "scope": "memory",
            "mode": mode,
            "indexed": indexed,
            "total": indexed,
        }
    )
    return {"ok": True, "scope": "memory", "indexed": indexed}


def _extract_discussion_field(body: str, field: str) -> str:
    match = re.search(rf"(?m)^{re.escape(field)}:[ \t]*([^\r\n]*)", body)
    return match.group(1).strip() if match else ""


def _split_discussion_csv(value: str) -> List[str]:
    if not value:
        return []
    return [part.strip() for part in value.split(",") if part.strip()]


def _parse_related_specs(value: str) -> List[str]:
    specs: List[str] = []
    for raw in _split_discussion_csv(value):
        cleaned = raw.strip()
        if cleaned.startswith("#"):
            cleaned = cleaned[1:]
        if cleaned.lower().startswith("spec-"):
            cleaned = cleaned[5:]
        if cleaned:
            specs.append(cleaned)
    return specs


def _load_discussion_documents(
    project_root: str,
    repo_hash: str = "",
) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Load the discussion log into discussion chunks.

    The source is the machine-local home work-notes file when present,
    otherwise the legacy repo-local ``.gwt/work/discussions.md`` (SPEC-3214).
    """
    source_path = _work_notes_source_path(project_root, repo_hash, "discussions.md")
    if not source_path.is_file():
        return [], []

    try:
        content = source_path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return [], []

    try:
        stat = source_path.stat()
    except OSError:
        return [], []
    manifest_entries = [
        {
            # The manifest key switches to the absolute home path after the
            # SPEC-3214 migration so the source change triggers one re-index.
            "path": str(source_path),
            "mtime": int(stat.st_mtime),
            "size": int(stat.st_size),
        }
    ]

    chunks = _chunk_spec_content(content)
    if not chunks:
        return [], manifest_entries

    discussions: List[Dict[str, Any]] = []
    grouped: Dict[tuple[str, str], int] = {}
    for chunk in chunks:
        heading = chunk["heading"]
        body = chunk["body"]
        if not heading.startswith("## "):
            continue

        date, title = _parse_memory_heading(heading)
        key = (date, title)
        chunk_idx = grouped.get(key, 0)
        grouped[key] = chunk_idx + 1
        digest_input = f"{heading}\n{body}".encode("utf-8")
        discussion_id = hashlib.sha1(digest_input).hexdigest()[:12]
        discussions.append(
            {
                "discussion_id": discussion_id,
                "date": date,
                "title": title,
                "status": _extract_discussion_field(body, "Status"),
                "topics": _split_discussion_csv(_extract_discussion_field(body, "Topics")),
                "related_specs": _parse_related_specs(
                    _extract_discussion_field(body, "Related SPECs")
                ),
                "related_works": _split_discussion_csv(
                    _extract_discussion_field(body, "Related Works")
                ),
                "promoted_to": _split_discussion_csv(
                    _extract_discussion_field(body, "Promoted To")
                ),
                "heading": heading,
                "body": body,
                "chunk_idx": chunk_idx,
                "total_chunks": 0,
            }
        )

    for entry in discussions:
        key = (entry["date"], entry["title"])
        entry["total_chunks"] = grouped[key]

    return discussions, manifest_entries


def _build_discussion_records(
    discussions: Sequence[Dict[str, Any]],
) -> List[Dict[str, Any]]:
    """Materialize Chroma upsert records for the discussions scope."""
    records: List[Dict[str, Any]] = []
    for entry in discussions:
        title = entry.get("title", "")
        heading = entry.get("heading", "")
        body = entry.get("body", "")
        document = f"{title}\n{heading}\n{body}".strip()
        records.append(
            {
                "id": f"discussion-{entry.get('discussion_id', '')}",
                "document": document,
                "metadata": {
                    "discussion_id": entry.get("discussion_id", ""),
                    "date": entry.get("date", ""),
                    "title": title,
                    "status": entry.get("status", ""),
                    "topics": ",".join(entry.get("topics", [])),
                    "related_specs": ",".join(entry.get("related_specs", [])),
                    "related_works": ",".join(entry.get("related_works", [])),
                    "promoted_to": ",".join(entry.get("promoted_to", [])),
                    "heading": heading,
                    "chunk_idx": int(entry.get("chunk_idx", 0)),
                    "total_chunks": int(entry.get("total_chunks", 1)),
                },
            }
        )
    return records


def action_index_discussions_v2(
    project_root: str,
    repo_hash: str,
    worktree_hash: Optional[str],
    mode: str = "full",
    db_root: Optional[Path] = None,
) -> dict:
    """Index ``.gwt/work/discussions.md`` into the repo-scoped discussions store."""
    del worktree_hash
    db_path = resolve_db_path(repo_hash, None, "discussions", db_root=db_root)
    discussions, new_entries = _load_discussion_documents(project_root, repo_hash)

    emit_progress(
        {
            "phase": "indexing",
            "scope": "discussions",
            "mode": mode,
            "done": 0,
            "total": len(discussions),
        }
    )

    indexed = 0
    build_store, staging = _full_build_store(db_path, mode)
    with acquire_lock(staging if staging is not None else db_path, exclusive=True):
        make_collection = (
            _make_chroma_collection
            if mode == "incremental"
            else _make_chroma_collection_repairing
        )
        client, collection = make_collection(build_store, V2_DISCUSSIONS_COLLECTION)
        try:
            old_entries = read_manifest(db_path, scope="discussions")
            diff = compute_manifest_diff(old_entries, new_entries)
            file_changed = bool(diff["added"] or diff["changed"] or diff["removed"])

            if mode != "incremental" or file_changed:
                try:
                    existing = collection.get()
                    if existing.get("ids"):
                        collection.delete(ids=existing["ids"])
                except Exception:
                    pass
                records = _build_discussion_records(discussions)
            else:
                records = []

            emit_progress(
                {
                    "phase": "diff",
                    "scope": "discussions",
                    "added": len(diff["added"]),
                    "changed": len(diff["changed"]),
                    "removed": len(diff["removed"]),
                }
            )

            if records:
                ids = [r["id"] for r in records]
                documents = [r["document"] for r in records]
                metadatas = [r["metadata"] for r in records]
                batch = 100
                for i in range(0, len(ids), batch):
                    collection.upsert(
                        ids=ids[i : i + batch],
                        documents=documents[i : i + batch],
                        metadatas=metadatas[i : i + batch],
                    )
                indexed = len(records)

            write_manifest(db_path, scope="discussions", entries=new_entries)
            _write_scope_meta(
                repo_hash=repo_hash,
                worktree_hash=None,
                scope="discussions",
                db_root=db_root,
                updates={
                    "last_repair_at": _now_utc().isoformat(),
                    "document_count": indexed,
                },
            )
        finally:
            _close_chroma_client(client)

    publish_error = _finish_full_build(db_path, staging, scope="discussions")
    if publish_error is not None:
        return publish_error

    emit_progress(
        {
            "phase": "complete",
            "scope": "discussions",
            "mode": mode,
            "indexed": indexed,
            "total": indexed,
        }
    )
    return {"ok": True, "scope": "discussions", "indexed": indexed}


def _gwt_home() -> Path:
    return Path(os.environ.get("HOME") or os.environ.get("USERPROFILE") or Path.home()) / ".gwt"


def _board_coordination_roots(repo_hash: str, project_root: Optional[str]) -> List[Path]:
    roots = [_gwt_home() / "projects" / repo_hash / "coordination"]
    if project_root:
        legacy = Path(project_root) / ".gwt" / "coordination"
        if legacy not in roots:
            roots.append(legacy)
    return roots


def _load_board_segment_events(segment_path: Path) -> List[Dict[str, Any]]:
    entries: List[Dict[str, Any]] = []
    try:
        lines = segment_path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return entries
    for line in lines:
        raw = line.strip()
        if not raw:
            continue
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            continue
        entry = payload.get("entry")
        if isinstance(entry, dict):
            entries.append(entry)
    return entries


def _load_board_documents(
    repo_hash: str,
    project_root: Optional[str],
) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Load Board entries from repo-scoped segmented coordination history."""
    for coordination_root in _board_coordination_roots(repo_hash, project_root):
        manifest_path = coordination_root / "events.manifest.json"
        segments_root = coordination_root / "events"
        if not manifest_path.is_file() or not segments_root.is_dir():
            continue
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        docs: List[Dict[str, Any]] = []
        manifest_entries: List[Dict[str, Any]] = []
        for segment in manifest.get("segments", []):
            if not isinstance(segment, dict):
                continue
            file_name = str(segment.get("file", "")).strip()
            if not file_name:
                continue
            segment_path = segments_root / file_name
            if not segment_path.is_file():
                continue
            try:
                stat = segment_path.stat()
            except OSError:
                continue
            manifest_entries.append(
                {
                    "path": f"coordination/events/{file_name}",
                    "mtime": int(stat.st_mtime),
                    "size": int(stat.st_size),
                }
            )
            for entry in _load_board_segment_events(segment_path):
                if "entry_id" not in entry and entry.get("id"):
                    entry["entry_id"] = entry.get("id")
                docs.append(entry)
        docs.sort(key=lambda entry: entry.get("created_at", ""))
        return docs, manifest_entries

    return [], []


def _board_entry_document(entry: Dict[str, Any]) -> str:
    parts = [
        str(entry.get("title_summary") or ""),
        str(entry.get("kind") or ""),
        str(entry.get("body") or ""),
        " ".join(str(value) for value in entry.get("related_topics") or []),
        " ".join(str(value) for value in entry.get("related_owners") or []),
        str(entry.get("author") or ""),
        str(entry.get("origin_branch") or ""),
    ]
    return "\n".join(part for part in parts if part).strip()


def _build_board_records(entries: Sequence[Dict[str, Any]]) -> List[Dict[str, Any]]:
    records: List[Dict[str, Any]] = []
    for entry in entries:
        entry_id = str(entry.get("id") or "").strip()
        if not entry_id:
            continue
        body = str(entry.get("body") or "")
        records.append(
            {
                "id": entry_id,
                "document": _board_entry_document(entry),
                "metadata": {
                    "entry_id": entry_id,
                    "kind": str(entry.get("kind") or ""),
                    "author": str(entry.get("author") or ""),
                    "title_summary": str(entry.get("title_summary") or ""),
                    "body_preview": body[:500],
                    "created_at": str(entry.get("created_at") or ""),
                    "updated_at": str(entry.get("updated_at") or ""),
                    "origin_branch": str(entry.get("origin_branch") or ""),
                    "origin_session_id": str(entry.get("origin_session_id") or ""),
                    "audience": ",".join(str(value) for value in entry.get("audience") or []),
                    "related_topics": ",".join(str(value) for value in entry.get("related_topics") or []),
                    "related_owners": ",".join(str(value) for value in entry.get("related_owners") or []),
                },
            }
        )
    return records


def action_index_board_v2(
    repo_hash: str,
    project_root: Optional[str],
    mode: str = "full",
    db_root: Optional[Path] = None,
) -> dict:
    """Index Board coordination history into the repo-scoped board store."""
    db_path = resolve_db_path(repo_hash, None, "board", db_root=db_root)
    entries, new_entries = _load_board_documents(repo_hash, project_root)

    emit_progress(
        {
            "phase": "indexing",
            "scope": "board",
            "mode": mode,
            "done": 0,
            "total": len(entries),
        }
    )

    indexed = 0
    build_store, staging = _full_build_store(db_path, mode)
    with acquire_lock(staging if staging is not None else db_path, exclusive=True):
        make_collection = (
            _make_chroma_collection
            if mode == "incremental"
            else _make_chroma_collection_repairing
        )
        client, collection = make_collection(build_store, V2_BOARD_COLLECTION)
        try:
            old_entries = read_manifest(db_path, scope="board")
            diff = compute_manifest_diff(old_entries, new_entries)
            history_changed = bool(diff["added"] or diff["changed"] or diff["removed"])

            if mode != "incremental" or history_changed:
                try:
                    existing = collection.get()
                    if existing.get("ids"):
                        collection.delete(ids=existing["ids"])
                except Exception:
                    pass
                records = _build_board_records(entries)
            else:
                records = []

            emit_progress(
                {
                    "phase": "diff",
                    "scope": "board",
                    "added": len(diff["added"]),
                    "changed": len(diff["changed"]),
                    "removed": len(diff["removed"]),
                }
            )

            if records:
                ids = [r["id"] for r in records]
                documents = [r["document"] for r in records]
                metadatas = [r["metadata"] for r in records]
                batch = 100
                for i in range(0, len(ids), batch):
                    collection.upsert(
                        ids=ids[i : i + batch],
                        documents=documents[i : i + batch],
                        metadatas=metadatas[i : i + batch],
                    )
                indexed = len(records)

            write_manifest(db_path, scope="board", entries=new_entries)
            _write_scope_meta(
                repo_hash=repo_hash,
                worktree_hash=None,
                scope="board",
                db_root=db_root,
                updates={
                    "last_repair_at": _now_utc().isoformat(),
                    "document_count": indexed,
                },
            )
        finally:
            _close_chroma_client(client)

    publish_error = _finish_full_build(db_path, staging, scope="board")
    if publish_error is not None:
        return publish_error

    emit_progress(
        {
            "phase": "complete",
            "scope": "board",
            "mode": mode,
            "indexed": indexed,
            "total": indexed,
        }
    )
    return {"ok": True, "scope": "board", "indexed": indexed}


# ---------------------------------------------------------------------
# v2 actions: index-works (SPEC-2359 US-80)
# ---------------------------------------------------------------------


def _work_project_state_dir(repo_hash: str) -> Path:
    return _gwt_home() / "projects" / repo_hash / "project-state"


def _fold_work_events_into_items(
    items_by_id: Dict[str, Dict[str, Any]],
    order: List[str],
    content: str,
) -> None:
    """Fold raw ``work-events.jsonl`` / ``events.jsonl`` lines into Work items.

    Mirrors the minimal subset of the Rust ``WorkItemsProjection`` fold needed
    for search documents: title/intent/summary/owner take the last non-empty
    value, status follows the latest event, execution containers accumulate,
    and board/related references are collected. The fold is order-tolerant —
    events are sorted by ``updated_at`` before applying — so it can absorb
    git union-merge artifacts and duplicate lines (dedup by event id).
    """
    events: List[Dict[str, Any]] = []
    for line in content.splitlines():
        raw = line.strip()
        if not raw:
            continue
        try:
            event = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict) and event.get("work_item_id"):
            events.append(event)

    seen_event_ids: set = set()
    for item in items_by_id.values():
        for ev in item.get("events", []):
            ev_id = ev.get("id") if isinstance(ev, dict) else None
            if ev_id:
                seen_event_ids.add(ev_id)

    events.sort(key=lambda ev: str(ev.get("updated_at") or ""))
    for event in events:
        event_id = event.get("id")
        if event_id and event_id in seen_event_ids:
            continue
        if event_id:
            seen_event_ids.add(event_id)
        work_id = str(event.get("work_item_id") or "").strip()
        if not work_id:
            continue
        item = items_by_id.get(work_id)
        if item is None:
            title = event.get("title") or event.get("intent") or work_id
            item = {
                "id": work_id,
                "title": str(title or work_id),
                "intent": event.get("intent") or "",
                "summary": event.get("summary") or "",
                "status_category": "",
                "owner": event.get("owner") or "",
                "execution_containers": [],
                "board_refs": [],
                "related_work_item_ids": [],
                "discarded": False,
                "events": [],
            }
            items_by_id[work_id] = item
            order.append(work_id)

        for field in ("title", "intent", "summary", "owner"):
            value = event.get(field)
            if isinstance(value, str) and value.strip():
                item[field] = value
        status = event.get("status_category")
        if isinstance(status, str) and status.strip():
            item["status_category"] = status
        elif event.get("kind") == "done":
            item["status_category"] = "done"
        if event.get("kind") == "discard":
            item["discarded"] = True
        container = event.get("execution_container")
        if isinstance(container, dict):
            item.setdefault("execution_containers", []).append(container)
        board_entry_id = event.get("board_entry_id")
        if isinstance(board_entry_id, str) and board_entry_id.strip():
            item.setdefault("board_refs", []).append(board_entry_id)
        related = event.get("related_work_item_id")
        if isinstance(related, str) and related.strip():
            item.setdefault("related_work_item_ids", []).append(related)


def _load_work_documents(
    repo_hash: str,
    project_root: Optional[str],
) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Load Work items from the home-scoped projection (primary) or event logs.

    Primary path: the home projection JSON written by the Rust Work surface
    (``~/.gwt/projects/<repo_hash>/project-state/works.json``; the legacy name
    ``work_items.json`` is also accepted). When that file is absent, fall back
    to folding ``project-state/work-events.jsonl`` and the repo-local
    ``<project_root>/.gwt/work/events.jsonl`` into items, mirroring the Rust
    projection build.

    ALL works are returned, including completed and discarded ones — finding a
    Work done a month ago is the whole point of the scope (SPEC-2359 US-80).

    Returns ``(works, manifest_entries)`` where ``manifest_entries`` describes
    each source file's mtime/size so incremental rebuilds can detect changes.
    """
    state_dir = _work_project_state_dir(repo_hash)
    manifest_entries: List[Dict[str, Any]] = []

    projection_path: Optional[Path] = None
    for name in ("works.json", "work_items.json"):
        candidate = state_dir / name
        if candidate.is_file():
            projection_path = candidate
            break

    if projection_path is not None:
        try:
            payload = json.loads(projection_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError, UnicodeDecodeError):
            payload = None
        if isinstance(payload, dict):
            works = [
                item
                for item in payload.get("work_items", [])
                if isinstance(item, dict) and str(item.get("id") or "").strip()
            ]
            try:
                stat = projection_path.stat()
                manifest_entries.append(
                    {
                        "path": f"project-state/{projection_path.name}",
                        "mtime": int(stat.st_mtime),
                        "size": int(stat.st_size),
                    }
                )
            except OSError:
                pass
            return works, manifest_entries

    # Fallback: fold the event logs into items.
    items_by_id: Dict[str, Dict[str, Any]] = {}
    order: List[str] = []
    event_sources: List[Path] = [state_dir / "work-events.jsonl"]
    if project_root:
        event_sources.append(
            Path(project_root) / ".gwt" / "work" / "events.jsonl"
        )

    for source in event_sources:
        if not source.is_file():
            continue
        try:
            content = source.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        _fold_work_events_into_items(items_by_id, order, content)
        try:
            stat = source.stat()
        except OSError:
            continue
        try:
            rel = source.relative_to(_gwt_home() / "projects" / repo_hash)
            path_label = rel.as_posix()
        except ValueError:
            path_label = ".gwt/work/events.jsonl"
        manifest_entries.append(
            {
                "path": path_label,
                "mtime": int(stat.st_mtime),
                "size": int(stat.st_size),
            }
        )

    works = [items_by_id[work_id] for work_id in order]
    return works, manifest_entries


def _work_entry_document(work: Dict[str, Any]) -> str:
    """Join the non-empty searchable fields of a Work into one document string.

    Includes title, intent, summary, owner, the branch name(s) of every
    execution container, and the linked PR / Issue identifiers (PR numbers /
    URLs from execution containers, plus board / related-work references).
    """
    parts: List[str] = [
        str(work.get("title") or ""),
        str(work.get("intent") or ""),
        str(work.get("summary") or ""),
        str(work.get("owner") or ""),
    ]
    for container in work.get("execution_containers") or []:
        if not isinstance(container, dict):
            continue
        branch = container.get("branch")
        if isinstance(branch, str) and branch.strip():
            parts.append(branch)
        pr_number = container.get("pr_number")
        if pr_number is not None:
            parts.append(f"#{pr_number}")
        pr_url = container.get("pr_url")
        if isinstance(pr_url, str) and pr_url.strip():
            parts.append(pr_url)
    for ref in work.get("board_refs") or []:
        if isinstance(ref, str) and ref.strip():
            parts.append(ref)
    for ref in work.get("related_work_item_ids") or []:
        if isinstance(ref, str) and ref.strip():
            parts.append(ref)
    return "\n".join(part for part in parts if part).strip()


def _work_branches(work: Dict[str, Any]) -> List[str]:
    branches: List[str] = []
    for container in work.get("execution_containers") or []:
        if not isinstance(container, dict):
            continue
        branch = container.get("branch")
        if isinstance(branch, str) and branch.strip() and branch not in branches:
            branches.append(branch)
    return branches


def _work_pr_numbers(work: Dict[str, Any]) -> List[str]:
    prs: List[str] = []
    for container in work.get("execution_containers") or []:
        if not isinstance(container, dict):
            continue
        pr_number = container.get("pr_number")
        if pr_number is not None:
            value = str(pr_number)
            if value not in prs:
                prs.append(value)
    return prs


def _build_work_records(works: Sequence[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Materialize Chroma upsert records for the works scope.

    The metadata carries the contract fields the Rust ``work_result()`` reads:
    ``work_id`` (required), ``title``, ``intent``, ``status``.
    """
    records: List[Dict[str, Any]] = []
    for work in works:
        work_id = str(work.get("id") or "").strip()
        if not work_id:
            continue
        status = str(work.get("status_category") or "")
        if work.get("discarded"):
            status = "discarded"
        records.append(
            {
                "id": f"work-{work_id}",
                "document": _work_entry_document(work),
                "metadata": {
                    "work_id": work_id,
                    "title": str(work.get("title") or ""),
                    "intent": str(work.get("intent") or ""),
                    "status": status,
                    "owner": str(work.get("owner") or ""),
                    "branches": ",".join(_work_branches(work)),
                    "pr_numbers": ",".join(_work_pr_numbers(work)),
                },
            }
        )
    return records


def action_index_works_v2(
    project_root: Optional[str],
    repo_hash: str,
    worktree_hash: Optional[str],
    mode: str = "full",
    db_root: Optional[Path] = None,
) -> dict:
    """Index past Work items into the repo-scoped works Chroma store.

    `worktree_hash` is accepted for symmetry with the other v2 actions but is
    ignored — Work history is repo-scoped. On first build, every existing Work
    item (including completed and discarded ones) is folded into a document so
    work done weeks ago remains discoverable (SPEC-2359 US-80 backfill).
    """
    del worktree_hash  # repo-scoped scope does not consume the worktree hash
    db_path = resolve_db_path(repo_hash, None, "works", db_root=db_root)
    works, new_entries = _load_work_documents(repo_hash, project_root)
    new_entries.sort(key=lambda entry: entry["path"])

    emit_progress(
        {
            "phase": "indexing",
            "scope": "works",
            "mode": mode,
            "done": 0,
            "total": len(works),
        }
    )

    indexed = 0
    build_store, staging = _full_build_store(db_path, mode)
    with acquire_lock(staging if staging is not None else db_path, exclusive=True):
        make_collection = (
            _make_chroma_collection
            if mode == "incremental"
            else _make_chroma_collection_repairing
        )
        client, collection = make_collection(build_store, V2_WORKS_COLLECTION)
        try:
            old_entries = read_manifest(db_path, scope="works")
            diff = compute_manifest_diff(old_entries, new_entries)
            source_changed = bool(diff["added"] or diff["changed"] or diff["removed"])

            if mode != "incremental" or source_changed:
                try:
                    existing = collection.get()
                    if existing.get("ids"):
                        collection.delete(ids=existing["ids"])
                except Exception:
                    pass
                records = _build_work_records(works)
            else:
                records = []

            emit_progress(
                {
                    "phase": "diff",
                    "scope": "works",
                    "added": len(diff["added"]),
                    "changed": len(diff["changed"]),
                    "removed": len(diff["removed"]),
                }
            )

            if records:
                ids = [r["id"] for r in records]
                documents = [r["document"] for r in records]
                metadatas = [r["metadata"] for r in records]
                batch = 100
                for i in range(0, len(ids), batch):
                    collection.upsert(
                        ids=ids[i : i + batch],
                        documents=documents[i : i + batch],
                        metadatas=metadatas[i : i + batch],
                    )
                indexed = len(records)

            write_manifest(db_path, scope="works", entries=new_entries)
            _write_scope_meta(
                repo_hash=repo_hash,
                worktree_hash=None,
                scope="works",
                db_root=db_root,
                updates={
                    "last_repair_at": _now_utc().isoformat(),
                    "document_count": indexed,
                },
            )
        finally:
            _close_chroma_client(client)

    publish_error = _finish_full_build(db_path, staging, scope="works")
    if publish_error is not None:
        return publish_error

    emit_progress(
        {
            "phase": "complete",
            "scope": "works",
            "mode": mode,
            "indexed": indexed,
            "total": indexed,
        }
    )
    return {"ok": True, "scope": "works", "indexed": indexed}


def _format_memory_results(
    items: List[Dict[str, Any]], n_results: int = 10
) -> List[Dict[str, Any]]:
    """Collapse chunked memory results so each (date, title) appears once."""
    formatted: List[Dict[str, Any]] = []
    seen: set = set()
    for it in items:
        meta = it.get("metadata") or {}
        date = meta.get("date", "")
        title = meta.get("title", "")
        key = (date, title)
        if key in seen:
            continue
        seen.add(key)
        formatted.append(
            _attach_match_fields(
                {
                    "date": date,
                    "title": title,
                    "heading": meta.get("heading", ""),
                    "chunk_idx": int(meta.get("chunk_idx", 0)),
                    "distance": it.get("distance"),
                },
                it,
            )
        )
        if len(formatted) >= n_results:
            break
    return formatted


def _format_discussion_results(
    items: List[Dict[str, Any]], n_results: int = 10
) -> List[Dict[str, Any]]:
    """Collapse chunked discussion results so each dated title appears once."""
    formatted: List[Dict[str, Any]] = []
    seen: set = set()
    for it in items:
        meta = it.get("metadata") or {}
        date = meta.get("date", "")
        title = meta.get("title", "")
        key = (date, title)
        if key in seen:
            continue
        seen.add(key)
        formatted.append(
            _attach_match_fields(
                {
                    "discussion_id": meta.get("discussion_id", it.get("id", "")),
                    "date": date,
                    "title": title,
                    "status": meta.get("status", ""),
                    "topics": _split_csv_meta(meta.get("topics", "")),
                    "related_specs": _split_csv_meta(meta.get("related_specs", "")),
                    "related_works": _split_csv_meta(meta.get("related_works", "")),
                    "promoted_to": _split_csv_meta(meta.get("promoted_to", "")),
                    "heading": meta.get("heading", ""),
                    "chunk_idx": int(meta.get("chunk_idx", 0)),
                    "distance": it.get("distance"),
                },
                it,
            )
        )
        if len(formatted) >= n_results:
            break
    return formatted


def _build_spec_records(specs: Sequence[Dict[str, Any]]) -> List[Dict[str, Any]]:
    records: List[Dict[str, Any]] = []
    for spec in specs:
        chunks = _chunk_spec_content(spec.get("content", ""))
        if not chunks:
            chunks = [{"heading": "(empty)", "body": ""}]
        total_chunks = len(chunks)
        for idx, chunk in enumerate(chunks):
            document = f"{spec.get('title', '')}\n{chunk['heading']}\n{chunk['body']}"
            records.append(
                {
                    "id": f"spec-{spec.get('spec_id', '')}:chunk-{idx}",
                    "document": document,
                    "metadata": {
                        "spec_id": spec.get("spec_id", ""),
                        "title": spec.get("title", ""),
                        "status": spec.get("status", ""),
                        "phase": spec.get("phase", ""),
                        "dir_name": spec.get("dir_name", ""),
                        "chunk_idx": idx,
                        "total_chunks": total_chunks,
                        "chunk_heading": chunk["heading"],
                    },
                }
            )
    return records


def _delete_spec_records(collection, spec_ids: Sequence[str]) -> None:
    if not spec_ids:
        return
    targets = {str(spec_id) for spec_id in spec_ids}
    try:
        existing = collection.get()
    except Exception:
        return

    ids = existing.get("ids") or []
    metadatas = existing.get("metadatas") or []
    to_delete: List[str] = []
    for idx, record_id in enumerate(ids):
        meta = metadatas[idx] if idx < len(metadatas) else {}
        spec_id = str((meta or {}).get("spec_id", ""))
        if not spec_id and record_id.startswith("spec-"):
            spec_id = record_id[5:].split(":chunk-", 1)[0]
        if spec_id in targets:
            to_delete.append(record_id)
    if to_delete:
        try:
            collection.delete(ids=to_delete)
        except Exception:
            pass


def _safe_collection_count(collection) -> int:
    try:
        return int(collection.count())
    except Exception:
        return 0


def _scope_meta_path(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
) -> Path:
    if scope == "specs":
        return resolve_db_path(repo_hash, None, "specs", db_root=db_root) / META_FILENAME
    if scope == "memory":
        return resolve_db_path(repo_hash, None, "memory", db_root=db_root) / META_FILENAME
    if scope == "discussions":
        return resolve_db_path(repo_hash, None, "discussions", db_root=db_root) / META_FILENAME
    if scope == "board":
        return resolve_db_path(repo_hash, None, "board", db_root=db_root) / META_FILENAME
    if scope == "works":
        return resolve_db_path(repo_hash, None, "works", db_root=db_root) / META_FILENAME
    if scope in WORKTREE_SCOPED:
        worktree_dir = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root).parent
        return worktree_dir / META_FILENAME
    raise ValueError(f"scope meta unsupported for {scope}")


def _read_scope_meta_blob(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
) -> Dict[str, Any]:
    meta_path = _scope_meta_path(repo_hash, worktree_hash, scope, db_root=db_root)
    if not meta_path.is_file():
        return {"schema_version": INDEX_SCHEMA_VERSION, "scopes": {}}
    try:
        payload = json.loads(meta_path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError, ValueError, UnicodeDecodeError):
        return {"schema_version": INDEX_SCHEMA_VERSION, "scopes": {}}
    if not isinstance(payload, dict):
        return {"schema_version": INDEX_SCHEMA_VERSION, "scopes": {}}
    scopes = payload.get("scopes")
    if not isinstance(scopes, dict):
        payload["scopes"] = {}
    payload.setdefault("schema_version", INDEX_SCHEMA_VERSION)
    return payload


def _read_scope_meta(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
) -> Dict[str, Any]:
    payload = _read_scope_meta_blob(repo_hash, worktree_hash, scope, db_root=db_root)
    scopes = payload.get("scopes") or {}
    scope_payload = scopes.get(scope, {})
    return scope_payload if isinstance(scope_payload, dict) else {}


def _write_scope_meta(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
    updates: Optional[Dict[str, Any]] = None,
) -> None:
    meta_path = _scope_meta_path(repo_hash, worktree_hash, scope, db_root=db_root)
    payload = _read_scope_meta_blob(repo_hash, worktree_hash, scope, db_root=db_root)
    payload["schema_version"] = INDEX_SCHEMA_VERSION
    scopes = payload.setdefault("scopes", {})
    scope_payload = scopes.get(scope, {})
    if not isinstance(scope_payload, dict):
        scope_payload = {}
    if updates:
        scope_payload.update(updates)
    scopes[scope] = scope_payload
    meta_path.parent.mkdir(parents=True, exist_ok=True)
    meta_path.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
    )


def _scope_collection_name(scope: str) -> str:
    return {
        "files": V2_FILES_CODE_COLLECTION,
        "files-docs": V2_FILES_DOCS_COLLECTION,
        "specs": V2_SPECS_COLLECTION,
        "issues": V2_ISSUES_COLLECTION,
        "memory": V2_MEMORY_COLLECTION,
        "discussions": V2_DISCUSSIONS_COLLECTION,
        "board": V2_BOARD_COLLECTION,
        "works": V2_WORKS_COLLECTION,
    }[scope]


def _legacy_residue_detected(
    repo_hash: str,
    worktree_hash: Optional[str],
    db_root: Optional[Path] = None,
) -> bool:
    if not worktree_hash:
        return False
    worktree_root = resolve_db_path(repo_hash, worktree_hash, "files", db_root=db_root).parent
    return worktree_root.joinpath("specs").exists() or worktree_root.joinpath("manifest-specs.json").exists()


def _close_chroma_client(client) -> None:
    try:
        client.close()
    except Exception:
        pass


def _scope_document_count(db_path: Path, scope: str) -> tuple[bool, int]:
    store = resolve_active_store(db_path)
    chroma_sqlite = store / "chroma.sqlite3"
    if not chroma_sqlite.exists():
        return False, 0
    try:
        with acquire_lock(db_path, exclusive=False):
            store = resolve_active_store(db_path)
            client, collection = _open_chroma_collection(store, _scope_collection_name(scope))
            try:
                return True, _safe_collection_count(collection)
            finally:
                _close_chroma_client(client)
    except Exception:
        return False, 0


def _scope_status_v2(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
) -> Dict[str, Any]:
    db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
    manifest_path = _manifest_path(db_path, scope)
    manifest_entries = read_manifest(db_path, scope=scope)
    manifest_count = len(manifest_entries)
    pointer_corrupt = _active_pointer_corrupt(db_path)
    exists = (resolve_active_store(db_path) / "chroma.sqlite3").exists()
    count_ok, document_count = _scope_document_count(db_path, scope)
    legacy_detected = _legacy_residue_detected(repo_hash, worktree_hash, db_root=db_root)
    scope_meta = _read_scope_meta(repo_hash, worktree_hash, scope, db_root=db_root)

    reason = "ready"
    healthy = True
    repair_required = False

    if pointer_corrupt:
        # Phase 70 FR-390: an unreadable active pointer must classify for
        # repair instead of silently reading an arbitrary store.
        reason = "active_pointer_corrupt"
        healthy = False
        repair_required = True
    elif not exists or not count_ok:
        reason = "collection_missing"
        healthy = False
        repair_required = True
    elif not manifest_path.is_file():
        reason = "manifest_missing"
        healthy = False
        repair_required = True
    elif legacy_detected:
        reason = "legacy_residue"
        healthy = False
        repair_required = True
    elif document_count == 0 and manifest_count > 0:
        reason = "empty_collection"
        healthy = False
        repair_required = True
    elif scope in ("specs", "memory", "discussions", "board", "works") and document_count < manifest_count:
        reason = "count_mismatch"
        healthy = False
        repair_required = True
    elif scope not in ("specs", "memory", "discussions", "board", "works") and document_count != manifest_count:
        reason = "empty_collection" if document_count == 0 and manifest_count > 0 else "count_mismatch"
        healthy = False
        repair_required = True

    return {
        "exists": exists,
        "healthy": healthy,
        "repair_required": repair_required,
        "document_count": document_count,
        "reason": reason,
        "legacy_residue_detected": legacy_detected,
        "last_repair_at": scope_meta.get("last_repair_at"),
    }


def _issue_status_v2(
    repo_hash: str,
    db_root: Optional[Path] = None,
) -> Dict[str, Any]:
    db_path = resolve_db_path(repo_hash, None, "issues", db_root=db_root)
    meta = _read_issue_meta(db_path) or {}
    source = _issue_cache_source_snapshot(repo_hash)
    exists = (resolve_active_store(db_path) / "chroma.sqlite3").exists() or (
        db_path / META_FILENAME
    ).is_file()
    count_ok, document_count = _scope_document_count(db_path, "issues")

    reason = "ready"
    healthy = True
    repair_required = False

    if not exists or not count_ok:
        reason = "collection_missing"
        healthy = False
        repair_required = True
    elif not meta:
        reason = "metadata_missing"
        healthy = False
        repair_required = True
    else:
        indexed_fingerprint = meta.get("source_cache_fingerprint")
        current_fingerprint = source.get("fingerprint")
        if current_fingerprint and indexed_fingerprint != current_fingerprint:
            reason = "source_cache_changed"
            healthy = False
            repair_required = True

    status: Dict[str, Any] = {
        "exists": exists,
        "healthy": healthy,
        "repair_required": repair_required,
        "document_count": document_count,
        "reason": reason,
        "legacy_residue_detected": False,
        "last_repair_at": meta.get("last_full_refresh"),
    }
    if meta:
        status.update(
            {
                "last_full_refresh": meta.get("last_full_refresh"),
                "ttl_minutes": meta.get("ttl_minutes", ISSUE_TTL_MINUTES_DEFAULT),
            }
        )
        if meta.get("source_cache_fingerprint"):
            status["source_cache_fingerprint"] = meta.get("source_cache_fingerprint")
        if "source_document_count" in meta:
            status["source_document_count"] = meta.get("source_document_count")
        if meta.get("source_cache_refresh_at"):
            status["source_cache_refresh_at"] = meta.get("source_cache_refresh_at")
        if reason == "source_cache_changed":
            status["current_source_cache_fingerprint"] = source.get("fingerprint")
            status["current_source_document_count"] = source.get("document_count")
        last = _parse_iso(meta.get("last_full_refresh", "")) if meta.get("last_full_refresh") else None
        if last is not None:
            age = (_now_utc() - last).total_seconds()
            ttl_secs = meta.get("ttl_minutes", ISSUE_TTL_MINUTES_DEFAULT) * 60
            status["ttl_remaining_seconds"] = max(0, int(ttl_secs - age))
    return status


def action_index_issues_v2(
    repo_hash: str,
    project_root: str,
    db_root: Optional[Path] = None,
    respect_ttl: bool = False,
    ttl_minutes: int = ISSUE_TTL_MINUTES_DEFAULT,
) -> dict:
    """Index GitHub Issues using the v2 layout. Respects TTL on demand."""
    db_path = resolve_db_path(repo_hash, None, "issues", db_root=db_root)

    if respect_ttl:
        meta = _read_issue_meta(db_path)
        if meta and meta.get("last_full_refresh"):
            last = _parse_iso(meta["last_full_refresh"])
            if last is not None:
                age = (_now_utc() - last).total_seconds()
                if age < ttl_minutes * 60:
                    emit_progress(
                        {
                            "phase": "skipped",
                            "scope": "issues",
                            "reason": "ttl",
                            "ttl_remaining_seconds": int(ttl_minutes * 60 - age),
                        }
                    )
                    return {
                        "ok": True,
                        "skipped": True,
                        "scope": "issues",
                        "ttl_remaining_seconds": int(ttl_minutes * 60 - age),
                    }

    emit_progress(
        {
            "phase": "indexing",
            "scope": "issues",
            "done": 0,
            "total": 0,
        }
    )

    staging = _staging_dir_for(db_path)
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True, exist_ok=True)
    with acquire_lock(staging, exclusive=True):
        issues = _load_cached_issue_documents(repo_hash)
        source = _issue_cache_source_snapshot(repo_hash)

        client, collection = _make_chroma_collection_repairing(staging, V2_ISSUES_COLLECTION)
        try:
            if issues:
                ids: List[str] = []
                documents: List[str] = []
                metadatas: List[Dict[str, Any]] = []
                for issue in issues:
                    number = issue.get("number", 0)
                    title = issue.get("title", "")
                    body = issue.get("body", "")
                    state = issue.get("state", "")
                    labels = issue.get("labels", [])
                    ids.append(str(number))
                    documents.append(f"{title}\n{body}")
                    metadatas.append(
                        {
                            "number": number,
                            "title": title,
                            "url": "",
                            "state": state,
                            "labels": ",".join(labels),
                        }
                    )
                batch = 100
                for i in range(0, len(ids), batch):
                    collection.upsert(
                        ids=ids[i : i + batch],
                        documents=documents[i : i + batch],
                        metadatas=metadatas[i : i + batch],
                    )

        finally:
            _close_chroma_client(client)

    def _commit_issue_meta():
        # Meta (TTL / source fingerprint) is only advanced once the new
        # generation is actually active (FR-390), inside the same
        # publication lock as the pointer swap.
        _write_issue_meta(
            db_path,
            {
                "schema_version": INDEX_SCHEMA_VERSION,
                "last_full_refresh": _now_utc().isoformat(),
                "ttl_minutes": ttl_minutes,
                "document_count": len(issues),
                "source_cache_fingerprint": source["fingerprint"],
                "source_document_count": source["document_count"],
                "source_cache_refresh_at": source.get("cache_refresh_at"),
            },
        )

    publish = _publish_generation(
        db_path,
        staging,
        scope="issues",
        document_count=len(issues),
        after_publish=_commit_issue_meta,
    )
    if not publish.get("ok"):
        return publish

    emit_progress(
        {
            "phase": "complete",
            "scope": "issues",
            "indexed": len(issues),
            "total": len(issues),
        }
    )
    return {"ok": True, "scope": "issues", "indexed": len(issues)}


# ---------------------------------------------------------------------
# v2 actions: search-* with auto-build fallback
# ---------------------------------------------------------------------


def _parse_required_terms(query: str) -> List[str]:
    terms: List[str] = []
    for match in re.finditer(r'"([^"]+)"|(\S+)', query):
        term = (match.group(1) if match.group(1) is not None else match.group(2)).strip()
        if term:
            terms.append(term)
    return terms


def _item_match_text(item: Dict[str, Any]) -> str:
    parts: List[str] = [str(item.get("id", "")), str(item.get("document", ""))]

    def collect(value: Any) -> None:
        if value is None:
            return
        if isinstance(value, dict):
            for nested in value.values():
                collect(nested)
            return
        if isinstance(value, (list, tuple, set)):
            for nested in value:
                collect(nested)
            return
        parts.append(str(value))

    collect(item.get("metadata") or {})
    return "\n".join(parts).casefold()


def _copy_with_match_fields(
    item: Dict[str, Any],
    match_mode: str,
    matched_terms: Sequence[str],
    missing_terms: Sequence[str],
) -> Dict[str, Any]:
    enriched = dict(item)
    enriched["match_mode"] = match_mode
    enriched["matched_terms"] = list(matched_terms)
    enriched["missing_terms"] = list(missing_terms)
    return enriched


def _apply_match_mode(
    items: List[Dict[str, Any]],
    query: str,
    match_mode: str,
) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    if match_mode != "all_terms":
        return items, []
    required_terms = _parse_required_terms(query)
    if not required_terms:
        return items, []

    strict: List[Dict[str, Any]] = []
    suggestions: List[Dict[str, Any]] = []
    for item in items:
        haystack = _item_match_text(item)
        matched = [term for term in required_terms if term.casefold() in haystack]
        missing = [term for term in required_terms if term.casefold() not in haystack]
        enriched = _copy_with_match_fields(item, match_mode, matched, missing)
        if missing:
            suggestions.append(enriched)
        else:
            strict.append(enriched)
    return strict, suggestions


def _attach_match_fields(result: Dict[str, Any], item: Dict[str, Any]) -> Dict[str, Any]:
    if item.get("match_mode"):
        result["match_mode"] = item.get("match_mode")
        result["matched_terms"] = list(item.get("matched_terms") or [])
        result["missing_terms"] = list(item.get("missing_terms") or [])
    return result


def _search_fetch_count(scope: str, n_results: int, match_mode: str) -> int:
    base = n_results * 5 if scope in ("specs", "memory", "discussions") else n_results
    if match_mode == "all_terms":
        return min(max(base, n_results * 5, 50), 200)
    return base


def _search_collection_v2(
    collection,
    query: str,
    n_results: int,
    query_embedding: Optional[List[float]] = None,
) -> List[Dict[str, Any]]:
    try:
        count = collection.count()
    except Exception:
        return []
    if count == 0:
        return []
    actual_n = min(n_results, count)
    if query_embedding is not None:
        # Phase 70 AS-2: reuse one query embedding across every scope in a
        # batch request instead of re-encoding per scope.
        results = collection.query(
            query_embeddings=[query_embedding],
            n_results=actual_n,
            include=["metadatas", "documents", "distances"],
        )
    else:
        results = collection.query(
            query_texts=[query],
            n_results=actual_n,
            include=["metadatas", "documents", "distances"],
        )
    items: List[Dict[str, Any]] = []
    if results and results.get("ids") and results["ids"][0]:
        for idx, doc_id in enumerate(results["ids"][0]):
            meta = results["metadatas"][0][idx] if results.get("metadatas") else {}
            distance = results["distances"][0][idx] if results.get("distances") else None
            items.append(
                {
                    "id": doc_id,
                    "metadata": meta,
                    "document": results["documents"][0][idx] if results.get("documents") else "",
                    "distance": round(distance, 4) if distance is not None else None,
                }
            )
    return items


def _format_file_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    formatted = []
    for it in items:
        formatted.append(
            _attach_match_fields(
                {
                    "path": (it["metadata"] or {}).get("path", it["id"]),
                    "description": (it["metadata"] or {}).get("description", ""),
                    "distance": it["distance"],
                    "fileType": (it["metadata"] or {}).get("file_type", ""),
                    "size": (it["metadata"] or {}).get("size", 0),
                },
                it,
            )
        )
    return formatted


def _format_spec_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Collapse chunked SPEC results so each spec_id appears only once.

    Items are delivered in distance order (lowest first), so the first
    occurrence of each spec_id is the best-scoring chunk for that SPEC.
    """
    formatted: List[Dict[str, Any]] = []
    seen_spec_ids: set = set()
    for it in items:
        meta = it["metadata"] or {}
        spec_id = meta.get("spec_id", it["id"])
        if spec_id in seen_spec_ids:
            continue
        seen_spec_ids.add(spec_id)
        formatted.append(
            _attach_match_fields(
                {
                    "spec_id": spec_id,
                    "title": meta.get("title", ""),
                    "status": meta.get("status", ""),
                    "phase": meta.get("phase", ""),
                    "dir_name": meta.get("dir_name", ""),
                    "distance": it["distance"],
                    "matched_section": meta.get("chunk_heading", ""),
                },
                it,
            )
        )
    return formatted


def _format_issue_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    formatted = []
    for it in items:
        meta = it["metadata"] or {}
        labels_raw = meta.get("labels", "")
        labels = [lb for lb in labels_raw.split(",") if lb] if labels_raw else []
        formatted.append(
            _attach_match_fields(
                {
                    "number": meta.get("number", it["id"]),
                    "title": meta.get("title", ""),
                    "url": meta.get("url", ""),
                    "state": meta.get("state", ""),
                    "labels": labels,
                    "distance": it["distance"],
                },
                it,
            )
        )
    return formatted


def _split_csv_meta(value: Any) -> List[str]:
    if not value:
        return []
    return [part for part in str(value).split(",") if part]


def _format_board_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    formatted = []
    for it in items:
        meta = it["metadata"] or {}
        formatted.append(
            _attach_match_fields(
                {
                    "entry_id": meta.get("entry_id", it["id"]),
                    "kind": meta.get("kind", ""),
                    "author": meta.get("author", ""),
                    "title_summary": meta.get("title_summary", ""),
                    "body_preview": meta.get("body_preview", ""),
                    "created_at": meta.get("created_at", ""),
                    "updated_at": meta.get("updated_at", ""),
                    "origin_branch": meta.get("origin_branch", ""),
                    "origin_session_id": meta.get("origin_session_id", ""),
                    "audience": _split_csv_meta(meta.get("audience", "")),
                    "related_topics": _split_csv_meta(meta.get("related_topics", "")),
                    "related_owners": _split_csv_meta(meta.get("related_owners", "")),
                    "distance": it["distance"],
                },
                it,
            )
        )
    return formatted


def _format_work_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Format works-scope hits, carrying the Rust ``work_result()`` contract.

    Each item exposes ``work_id`` (required — Rust drops items without it),
    ``title``, ``intent``, ``status``, the standard ``distance``, and the
    optional ``matched_terms`` / ``missing_terms`` arrays via
    ``_attach_match_fields``. Items missing a ``work_id`` are dropped to honor
    the contract on the Python side too.
    """
    formatted = []
    for it in items:
        meta = it["metadata"] or {}
        work_id = meta.get("work_id", it["id"])
        if not work_id:
            continue
        formatted.append(
            _attach_match_fields(
                {
                    "work_id": work_id,
                    "title": meta.get("title", ""),
                    "intent": meta.get("intent", ""),
                    "status": meta.get("status", ""),
                    "owner": meta.get("owner", ""),
                    "branches": _split_csv_meta(meta.get("branches", "")),
                    "pr_numbers": _split_csv_meta(meta.get("pr_numbers", "")),
                    "distance": it["distance"],
                },
                it,
            )
        )
    return formatted


def _scope_result_key(scope: str) -> str:
    return {
        "files": "results",
        "files-docs": "results",
        "specs": "specResults",
        "issues": "issueResults",
        "memory": "memoryResults",
        "discussions": "discussionResults",
        "board": "boardResults",
        "works": "workResults",
    }[scope]


def _format_scope_results(
    scope: str,
    items: List[Dict[str, Any]],
    n_results: int,
) -> List[Dict[str, Any]]:
    if scope in ("files", "files-docs"):
        return _format_file_results(items)[:n_results]
    if scope == "specs":
        return _format_spec_results(items)[:n_results]
    if scope == "memory":
        return _format_memory_results(items, n_results)
    if scope == "discussions":
        return _format_discussion_results(items, n_results)
    if scope == "board":
        return _format_board_results(items)[:n_results]
    if scope == "works":
        return _format_work_results(items)[:n_results]
    return _format_issue_results(items)[:n_results]


def _empty_corpus_diagnostic(scope: str, repo_hash: str) -> dict:
    """Build the non-OK payload for an unpopulated issues/specs search corpus.

    Returned by :func:`action_search_v2` when an auto-build search finds an
    empty index that the issue cache cannot explain (Issue #2979). The message
    is agent-facing: it states the empty result is a tooling failure, points at
    the cache directory, and tells the caller to refresh rather than treat the
    empty list as "no existing SPEC/Issue".
    """
    cache_dir = _issue_cache_root(repo_hash)
    noun = "SPECs" if scope == "specs" else "Issues"
    return {
        "ok": False,
        "error_code": "EMPTY_CORPUS",
        "scope": scope,
        "indexed": 0,
        "issue_cache_dir": str(cache_dir),
        "issue_cache_populated": False,
        "error": (
            f"{scope} search corpus is empty: the GitHub Issue cache at "
            f"{cache_dir} holds no cached issues for repo-hash {repo_hash}. "
            "This is a tooling failure (the cache was never synced for this "
            "repository, or the repo-hash does not match the populated cache), "
            f"not proof that the repository has no {noun}. Refresh the cache "
            "(open the project in the gwt GUI to sync, or run a gwtd issue "
            "sync) and retry the search before concluding no owner exists."
        ),
    }


def action_search_v2(
    action: str,
    repo_hash: str,
    worktree_hash: Optional[str],
    project_root: Optional[str],
    query: str,
    n_results: int = 10,
    no_auto_build: bool = False,
    db_root: Optional[Path] = None,
    match_mode: str = "semantic",
) -> dict:
    """Unified v2 search dispatcher with auto-build fallback."""
    scope_for_action = {
        "search-files": "files",
        "search-files-docs": "files-docs",
        "search-specs": "specs",
        "search-issues": "issues",
        "search-memory": "memory",
        "search-discussions": "discussions",
        "search-board": "board",
        "search-works": "works",
    }
    if action not in scope_for_action:
        return {"ok": False, "error_code": "BAD_ARGS", "error": f"unknown action {action}"}
    scope = scope_for_action[action]

    db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
    if scope == "issues":
        health = _issue_status_v2(repo_hash, db_root=db_root)
    else:
        health = _scope_status_v2(repo_hash, worktree_hash, scope, db_root=db_root)

    needs_build = not health["exists"] or health.get("repair_required", False)
    needs_spec_refresh = scope == "specs" and health.get("healthy", False) and not no_auto_build

    if needs_build or needs_spec_refresh:
        if no_auto_build and needs_build:
            error_code = "INDEX_UNHEALTHY" if health["exists"] else "INDEX_MISSING"
            return {
                "ok": False,
                "error_code": error_code,
                "error": (
                    f"index unhealthy at {db_path}: {health.get('reason', 'repair_required')}"
                    if error_code == "INDEX_UNHEALTHY"
                    else f"index not found at {db_path}"
                ),
                "status": health,
            }
        if project_root is None:
            return {
                "ok": False,
                "error_code": "BAD_ARGS",
                "error": "project_root required for auto-build",
            }
        emit_progress({"phase": "indexing", "scope": scope, "done": 0, "total": 0})
        if scope == "issues":
            build = action_index_issues_v2(
                repo_hash=repo_hash,
                project_root=project_root,
                db_root=db_root,
                respect_ttl=False,
            )
        elif scope == "specs":
            build = action_index_specs_v2(
                project_root=project_root,
                repo_hash=repo_hash,
                worktree_hash=worktree_hash or "",
                mode="full" if needs_build else "incremental",
                db_root=db_root,
            )
        elif scope == "memory":
            build = action_index_memory_v2(
                project_root=project_root,
                repo_hash=repo_hash,
                worktree_hash=None,
                mode="full",
                db_root=db_root,
            )
        elif scope == "discussions":
            build = action_index_discussions_v2(
                project_root=project_root,
                repo_hash=repo_hash,
                worktree_hash=None,
                mode="full",
                db_root=db_root,
            )
        elif scope == "board":
            build = action_index_board_v2(
                repo_hash=repo_hash,
                project_root=project_root,
                mode="full",
                db_root=db_root,
            )
        elif scope == "works":
            build = action_index_works_v2(
                project_root=project_root,
                repo_hash=repo_hash,
                worktree_hash=None,
                mode="full",
                db_root=db_root,
            )
        else:
            build = action_index_files_v2(
                project_root=project_root,
                repo_hash=repo_hash,
                worktree_hash=worktree_hash or "",
                mode="full",
                db_root=db_root,
                scope=scope,
            )
        if not build.get("ok"):
            return build
        emit_progress({"phase": "complete", "scope": scope, "total": build.get("indexed", 0)})

    with acquire_lock(db_path, exclusive=False):
        store = resolve_active_store(db_path)
        client, collection = _open_chroma_collection(store, _scope_collection_name(scope))
        try:
            collection_count = _safe_collection_count(collection)
            fetch_n = _search_fetch_count(scope, n_results, match_mode)
            items = _search_collection_v2(collection, query, fetch_n)
        finally:
            _close_chroma_client(client)

    # Issue #2979: an issues/specs search whose corpus is empty *because the
    # source issue cache was never populated for this repo-hash* must not
    # silently report `ok: true` with no results — agents read that empty list
    # as proof that no SPEC/Issue owner exists and create duplicates. Surface a
    # diagnostic so the failure is visible. This is gated to the agent
    # auto-build preflight (no_auto_build is False); the interactive GUI search
    # (search-multi, no_auto_build=True) keeps returning empty results so one
    # empty scope never fails the whole multi-scope search. A populated cache
    # that simply has no matching SPECs is a legitimate empty result and is left
    # untouched.
    if (
        scope in ("issues", "specs")
        and not no_auto_build
        and collection_count == 0
        and not _issue_cache_is_populated(repo_hash)
    ):
        return _empty_corpus_diagnostic(scope, repo_hash)

    strict_items, suggestion_items = _apply_match_mode(items, query, match_mode)
    payload = {
        "ok": True,
        _scope_result_key(scope): _format_scope_results(scope, strict_items, n_results),
    }
    if match_mode == "all_terms":
        payload["suggestions"] = _format_scope_results(scope, suggestion_items, n_results)
    return payload


def _classify_scope_for_search(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    db_root: Optional[Path] = None,
) -> tuple:
    """Map scope health to the Phase 70 search state (FR-387 / FR-388).

    - ``missing``: store was never built.
    - ``corrupt``: store exists but needs repair before it can be trusted.
    - ``stale``: verified store is intact but its source moved on (TTL
      expiry or source cache drift) — serve it and queue a refresh.
    - ``fresh``: healthy and current.
    """
    if scope == "issues":
        health = _issue_status_v2(repo_hash, db_root=db_root)
        if not health.get("exists"):
            return "missing", health
        if health.get("healthy"):
            if health.get("ttl_remaining_seconds") == 0:
                return "stale", health
            return "fresh", health
        if health.get("reason") == "source_cache_changed":
            return "stale", health
        return "corrupt", health
    health = _scope_status_v2(repo_hash, worktree_hash, scope, db_root=db_root)
    if not health.get("exists"):
        return "missing", health
    if health.get("repair_required"):
        return "corrupt", health
    return "fresh", health


def _search_scope_collection(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,
    query: str,
    n_results: int,
    match_mode: str,
    db_root: Optional[Path],
    query_embedding: Optional[List[float]],
) -> Dict[str, Any]:
    """Query one verified scope store (no health gating, no auto-build)."""
    db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
    with acquire_lock(db_path, exclusive=False):
        store = resolve_active_store(db_path)
        client, collection = _open_chroma_collection(store, _scope_collection_name(scope))
        try:
            fetch_n = _search_fetch_count(scope, n_results, match_mode)
            items = _search_collection_v2(
                collection, query, fetch_n, query_embedding=query_embedding
            )
        finally:
            _close_chroma_client(client)
    strict_items, suggestion_items = _apply_match_mode(items, query, match_mode)
    payload = {
        _scope_result_key(scope): _format_scope_results(scope, strict_items, n_results),
    }
    if match_mode == "all_terms":
        payload["suggestions"] = _format_scope_results(scope, suggestion_items, n_results)
    return payload


def action_search_multi_v2(
    repo_hash: str,
    worktree_hash: Optional[str],
    project_root: Optional[str],
    query: str,
    n_results: int,
    scopes: Sequence[str],
    db_root: Optional[Path] = None,
    match_mode: str = "semantic",
) -> Dict[str, Any]:
    """Versioned batch search across v2 scopes (Phase 70 FR-384).

    One process, one model load, one query encode for every requested scope
    (AS-2). Each scope is classified (fresh / stale / missing / corrupt)
    before searching: fresh and stale scopes are served from their verified
    stores, broken scopes are reported in ``scopes`` instead of failing the
    whole batch or silently returning empty results (FR-387 / FR-388). The
    Rust caller owns repair scheduling and the stale refresh queue.
    """
    valid_scopes = {
        "issues",
        "specs",
        "memory",
        "discussions",
        "board",
        "works",
        "files",
        "files-docs",
    }
    payload: Dict[str, Any] = {"ok": True}
    scope_states: Dict[str, Dict[str, Any]] = {}
    scope_results: Dict[str, Any] = {}
    stale_scopes: List[str] = []
    query_embedding: Optional[List[float]] = None
    for scope in scopes:
        if scope not in valid_scopes:
            return {
                "ok": False,
                "error_code": "BAD_ARGS",
                "error": f"unsupported search scope {scope}",
            }
        scope_worktree = worktree_hash if scope in WORKTREE_SCOPED else None
        state, health = _classify_scope_for_search(
            repo_hash, scope_worktree, scope, db_root=db_root
        )
        if state in ("missing", "corrupt"):
            scope_states[scope] = {
                "state": state,
                "reason": health.get("reason", state),
            }
            continue
        if query_embedding is None:
            query_embedding = E5EmbeddingFunction().embed_query([query])[0]
        try:
            result = _search_scope_collection(
                repo_hash,
                scope_worktree,
                scope,
                query,
                n_results,
                match_mode,
                db_root,
                query_embedding,
            )
        except Exception as error:  # store broke between classify and query
            scope_states[scope] = {"state": "corrupt", "reason": str(error)}
            continue
        scope_states[scope] = {"state": state}
        if state == "stale":
            stale_scopes.append(scope)
        scope_results[scope] = result
        for key, value in result.items():
            if key == "suggestions":
                payload.setdefault("suggestions", {})[scope] = value
            else:
                payload[key] = value
    payload["scopes"] = scope_states
    payload["scope_results"] = scope_results
    if stale_scopes:
        payload["stale_scopes"] = stale_scopes
    return payload


# ---------------------------------------------------------------------
# v2 status
# ---------------------------------------------------------------------


def _runtime_status_v2() -> Dict[str, Any]:
    try:
        asset_hash = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()[:16]
    except OSError:
        asset_hash = ""
    return {
        "healthy": True,
        "repaired": False,
        "reason": "ready",
        "asset_hash": asset_hash,
        "smoke_test": "passed",
    }


def action_status_v2(
    repo_hash: str,
    worktree_hash: Optional[str],
    db_root: Optional[Path] = None,
    worktree_hashes: Optional[Sequence[str]] = None,
) -> dict:
    out: Dict[str, Any] = {
        "issues": _issue_status_v2(repo_hash, db_root=db_root),
        "specs": _scope_status_v2(repo_hash, None, "specs", db_root=db_root),
        "memory": _scope_status_v2(repo_hash, None, "memory", db_root=db_root),
        "discussions": _scope_status_v2(repo_hash, None, "discussions", db_root=db_root),
        "board": _scope_status_v2(repo_hash, None, "board", db_root=db_root),
        "works": _scope_status_v2(repo_hash, None, "works", db_root=db_root),
    }
    if worktree_hash:
        for scope in ("files", "files-docs"):
            out[scope] = _scope_status_v2(repo_hash, worktree_hash, scope, db_root=db_root)

    payload = {"ok": True, "runtime": _runtime_status_v2(), "status": out}
    if worktree_hashes:
        # Phase 70 FR-393 / AS-13: one batch process reports every requested
        # worktree so all-worktree status no longer spawns one Python per
        # worktree.
        payload["worktrees"] = {
            hash_value: {
                scope: _scope_status_v2(
                    repo_hash, hash_value, scope, db_root=db_root
                )
                for scope in ("files", "files-docs")
            }
            for hash_value in worktree_hashes
            if hash_value
        }
    return payload


# =====================================================================
# Argparse + main
# =====================================================================


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="gwt ChromaDB index helper")
    parser.add_argument(
        "--action",
        required=True,
        choices=[
            "probe",
            "index-files",
            "index-files-docs",
            "search-files",
            "search-files-docs",
            "index",
            "search",
            "status",
            "index-issues",
            "search-issues",
            "index-specs",
            "search-specs",
            "index-memory",
            "search-memory",
            "index-discussions",
            "search-discussions",
            "index-board",
            "search-board",
            "index-works",
            "search-works",
            "search-multi",
        ],
    )
    parser.add_argument("--project-root", default="")
    parser.add_argument("--db-path", default="")
    parser.add_argument("--query", default="")
    parser.add_argument("--n-results", type=int, default=10)
    # Phase 8 flags
    parser.add_argument("--repo-hash", dest="repo_hash", default="")
    parser.add_argument("--worktree-hash", dest="worktree_hash", default="")
    parser.add_argument(
        "--scope",
        default="",
        choices=["", "issues", "specs", "files", "files-docs", "memory", "discussions", "board", "works"],
    )
    parser.add_argument("--scopes", default="")
    parser.add_argument("--match-mode", default="semantic", choices=["semantic", "all_terms"])
    parser.add_argument("--mode", default="full", choices=["full", "incremental"])
    parser.add_argument("--no-auto-build", dest="no_auto_build", action="store_true")
    parser.add_argument("--respect-ttl", dest="respect_ttl", action="store_true")
    # Phase 70 (Issue #3264): QoS profile for thread caps / process priority.
    parser.add_argument(
        "--qos",
        default=None,
        choices=["background", "interactive"],
    )
    # Explicit index root override (defaults to ~/.gwt/index).
    parser.add_argument("--db-root", dest="db_root", default="")
    # Phase 70 AS-13: batch all-worktree status in one process.
    parser.add_argument("--worktree-hashes", dest="worktree_hashes", default="")
    return parser.parse_args()


def _dispatch_v2(action: str, args: argparse.Namespace) -> int:
    """Phase 8 v2 dispatcher."""
    repo_hash = args.repo_hash
    worktree_hash = args.worktree_hash or None
    db_root = Path(args.db_root) if getattr(args, "db_root", "") else None

    try:
        if action == "status":
            worktree_hashes = [
                value.strip()
                for value in (args.worktree_hashes or "").split(",")
                if value.strip()
            ]
            emit(
                action_status_v2(
                    repo_hash,
                    worktree_hash,
                    db_root=db_root,
                    worktree_hashes=worktree_hashes or None,
                )
            )
            return 0

        if action == "index-issues":
            if not args.project_root:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--project-root is required"})
                return 2
            emit(
                action_index_issues_v2(
                    repo_hash=repo_hash,
                    project_root=args.project_root,
                    respect_ttl=args.respect_ttl,
                    db_root=db_root,
                )
            )
            return 0

        if action in ("index-files", "index-files-docs"):
            if not args.project_root:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--project-root is required"})
                return 2
            scope = args.scope or ("files-docs" if action == "index-files-docs" else "files")
            emit(
                action_index_files_v2(
                    project_root=args.project_root,
                    repo_hash=repo_hash,
                    worktree_hash=worktree_hash or "",
                    mode=args.mode,
                    scope=scope,
                    qos=args.qos or default_qos_for_action(action),
                    db_root=db_root,
                )
            )
            return 0

        if action == "index-specs":
            if not args.project_root:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--project-root is required"})
                return 2
            emit(
                action_index_specs_v2(
                    project_root=args.project_root,
                    repo_hash=repo_hash,
                    worktree_hash=worktree_hash or "",
                    mode=args.mode,
                    db_root=db_root,
                )
            )
            return 0

        if action == "index-memory":
            if not args.project_root:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--project-root is required"})
                return 2
            emit(
                action_index_memory_v2(
                    project_root=args.project_root,
                    repo_hash=repo_hash,
                    worktree_hash=None,
                    mode=args.mode,
                    db_root=db_root,
                )
            )
            return 0

        if action == "index-discussions":
            if not args.project_root:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--project-root is required"})
                return 2
            emit(
                action_index_discussions_v2(
                    project_root=args.project_root,
                    repo_hash=repo_hash,
                    worktree_hash=None,
                    mode=args.mode,
                    db_root=db_root,
                )
            )
            return 0

        if action == "index-board":
            emit(
                action_index_board_v2(
                    repo_hash=repo_hash,
                    project_root=args.project_root or None,
                    mode=args.mode,
                    db_root=db_root,
                )
            )
            return 0

        if action == "index-works":
            emit(
                action_index_works_v2(
                    project_root=args.project_root or None,
                    repo_hash=repo_hash,
                    worktree_hash=None,
                    mode=args.mode,
                    db_root=db_root,
                )
            )
            return 0

        if action == "search-multi":
            if not args.query:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--query is required"})
                return 2
            scopes = [scope.strip() for scope in args.scopes.split(",") if scope.strip()]
            emit(
                action_search_multi_v2(
                    repo_hash=repo_hash,
                    worktree_hash=worktree_hash,
                    project_root=args.project_root or None,
                    query=args.query,
                    n_results=args.n_results,
                    scopes=scopes,
                    match_mode=args.match_mode,
                    db_root=db_root,
                )
            )
            return 0

        if action in (
            "search-files",
            "search-files-docs",
            "search-specs",
            "search-issues",
            "search-memory",
            "search-discussions",
            "search-board",
            "search-works",
        ):
            if not args.query:
                emit({"ok": False, "error_code": "BAD_ARGS", "error": "--query is required"})
                return 2
            emit(
                action_search_v2(
                    action=action,
                    repo_hash=repo_hash,
                    worktree_hash=worktree_hash,
                    project_root=args.project_root or None,
                    query=args.query,
                    n_results=args.n_results,
                    no_auto_build=args.no_auto_build,
                    match_mode=args.match_mode,
                    db_root=db_root,
                )
            )
            return 0

        emit({"ok": False, "error_code": "BAD_ARGS", "error": f"unknown v2 action {action}"})
        return 2
    except Exception as exc:
        emit({"ok": False, "error_code": "RUNTIME_ERROR", "error": str(exc)})
        return 1


def main() -> int:
    args = parse_args()

    try:
        action = {
            "index": "index-files",
            "search": "search-files",
        }.get(args.action, args.action)

        # Phase 70 FR-385: thread caps / priority must be in place before any
        # action can trigger the lazy model import.
        configure_qos_threads(args.qos or default_qos_for_action(action))

        # Issue #2933: when the caller omitted --repo-hash (e.g. an agent pane
        # whose launch env did not export GWT_REPO_HASH / GWT_WORKTREE_HASH)
        # but did pass --project-root, derive the hashes here so the v2 search
        # pipeline engages instead of failing with "--db-path is required".
        if not args.repo_hash and args.project_root:
            derived = _derive_hashes_from_project_root(args.project_root)
            if derived:
                args.repo_hash = derived["repo_hash"]
                if not args.worktree_hash:
                    args.worktree_hash = derived["worktree_hash"]

        # Phase 8: when --repo-hash is supplied, dispatch the v2 action layer.
        if args.repo_hash:
            return _dispatch_v2(action, args)

        if action == "probe":
            emit(action_probe())
            return 0

        if action == "index-files":
            if not args.project_root:
                emit({"ok": False, "error": "--project-root is required for index-files"})
                return 2
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for index-files"})
                return 2
            emit(action_index(args.project_root, args.db_path))
            return 0

        if action == "search-files":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for search-files"})
                return 2
            if not args.query:
                emit({"ok": False, "error": "--query is required for search-files"})
                return 2
            emit(action_search(args.db_path, args.query, args.n_results))
            return 0

        if action == "search-files-docs":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for search-files-docs"})
                return 2
            if not args.query:
                emit({"ok": False, "error": "--query is required for search-files-docs"})
                return 2
            emit(action_search_docs(args.db_path, args.query, args.n_results))
            return 0

        if action == "status":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for status"})
                return 2
            emit(action_status(args.db_path))
            return 0

        if action == "index-issues":
            if not args.project_root:
                emit({"ok": False, "error": "--project-root is required for index-issues"})
                return 2
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for index-issues"})
                return 2
            emit(action_index_issues(args.project_root, args.db_path))
            return 0

        if action == "search-issues":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for search-issues"})
                return 2
            if not args.query:
                emit({"ok": False, "error": "--query is required for search-issues"})
                return 2
            emit(action_search_issues(args.db_path, args.query, args.n_results))
            return 0

        if action == "index-specs":
            if not args.project_root:
                emit({"ok": False, "error": "--project-root is required for index-specs"})
                return 2
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for index-specs"})
                return 2
            emit(action_index_specs(args.project_root, args.db_path))
            return 0

        if action == "search-specs":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for search-specs"})
                return 2
            if not args.query:
                emit({"ok": False, "error": "--query is required for search-specs"})
                return 2
            emit(action_search_specs(args.db_path, args.query, args.n_results))
            return 0

        emit({"ok": False, "error": f"Unsupported action: {action}"})
        return 2

    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
