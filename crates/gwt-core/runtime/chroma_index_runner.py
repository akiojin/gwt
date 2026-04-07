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


DEFAULT_IGNORE_PATTERNS = [
    ".git",
    ".gwt/index",
    "node_modules",
    "__pycache__",
    ".DS_Store",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "*.pyc",
    "*.pyo",
    "*.so",
    "*.dylib",
    "*.dll",
    "*.exe",
    "*.o",
    "*.a",
    "*.class",
    "*.jar",
    "*.war",
    "*.wasm",
    "*.min.js",
    "*.min.css",
    "*.map",
    "*.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "Cargo.lock",
]

BINARY_EXTENSIONS = {
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg",
    ".woff", ".woff2", ".ttf", ".eot", ".otf",
    ".pdf", ".zip", ".tar", ".gz", ".bz2", ".xz", ".7z",
    ".mp3", ".mp4", ".wav", ".avi", ".mov", ".mkv",
    ".dmg", ".msi", ".deb", ".rpm", ".AppImage",
    ".safetensors", ".bin", ".onnx", ".pt", ".pth",
}

MAX_FILE_SIZE = 1_048_576  # 1 MiB
CODE_COLLECTION = "files_code"
DOC_COLLECTION = "files_docs"
LEGACY_FILE_COLLECTION = "files"

SKIP_FILE_EXTENSIONS = {
    ".snap",
}

SKIP_ROOT_DIRECTORIES = {
    ".claude",
    ".codex",
    "specs",
    "specs-archive",
    "tasks",
}

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


def load_gitignore_patterns(project_root: Path) -> List[str]:
    """Load patterns from .gitignore."""
    gitignore = project_root / ".gitignore"
    patterns: List[str] = list(DEFAULT_IGNORE_PATTERNS)
    if gitignore.is_file():
        for line in gitignore.read_text(errors="replace").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                patterns.append(line)
    return patterns


def _pattern_to_regex(pattern: str) -> Optional[re.Pattern]:
    """Convert a simplified gitignore-style pattern to a regex."""
    negated = pattern.startswith("!")
    if negated:
        return None

    pattern = pattern.rstrip("/")
    pattern = pattern.lstrip("/")
    if not pattern:
        return None

    regex = pattern.replace(".", r"\.")
    regex = regex.replace("**", "{{GLOBSTAR}}")
    regex = regex.replace("*", "[^/]*")
    regex = regex.replace("{{GLOBSTAR}}", ".*")
    regex = regex.replace("?", "[^/]")

    if "/" not in pattern:
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
    """Recursively collect project files, respecting .gitignore."""
    patterns = load_gitignore_patterns(project_root)
    compiled = [p for p in (_pattern_to_regex(pat) for pat in patterns) if p is not None]
    result = []
    for root, dirs, files in os.walk(project_root):
        root_path = Path(root)
        rel_root = root_path.relative_to(project_root)
        rel_root_str = str(rel_root) if str(rel_root) != "." else ""

        dirs[:] = [
            d for d in dirs
            if not should_ignore(
                f"{rel_root_str}/{d}" if rel_root_str else d,
                compiled,
            )
        ]

        for fname in files:
            rel = f"{rel_root_str}/{fname}" if rel_root_str else fname
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
        content = file_path.read_text(errors="replace")
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


def action_search(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search implementation-focused project files."""
    return _search_file_collection(
        db_path,
        query,
        n_results,
        CODE_COLLECTION,
        "Collection 'files_code' not found. Run index-files first.",
    )


def action_search_docs(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search project docs kept separate from implementation files."""
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
    """Search the GitHub Issues index."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
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
    """Index local SPEC directories into ChromaDB collection 'specs'."""
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
            meta = json.loads(metadata_file.read_text(errors="replace"))
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
                spec_content = spec_md.read_text(errors="replace")[:500]
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
    """Search the local SPEC index."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
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

V2_SCOPES = ("issues", "specs", "files", "files-docs")
WORKTREE_SCOPED = {"specs", "files", "files-docs"}

V2_FILES_CODE_COLLECTION = "files_code"
V2_FILES_DOCS_COLLECTION = "files_docs"
V2_SPECS_COLLECTION = "specs"
V2_ISSUES_COLLECTION = "issues"


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

    if scope == "issues":
        return repo_dir / "issues"

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
        with portalocker.Lock(str(lock_path), mode="a+", flags=flag) as fh:
            try:
                yield
            finally:
                try:
                    fh.flush()
                except Exception:
                    pass
        return
    except ImportError:
        pass

    # Fallback path: fcntl on POSIX, msvcrt on Windows.
    if os.name == "nt":
        import msvcrt  # type: ignore

        fh = open(lock_path, "a+")
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

    fh = open(lock_path, "a+")
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
    if worktree_dir.name in ("specs", "files", "files-docs", "issues"):
        return worktree_dir.parent / f"manifest-{scope}.json"
    return worktree_dir / f"manifest-{scope}.json"


def read_manifest(worktree_dir: Path, scope: str) -> List[Dict[str, Any]]:
    """Read the manifest for the given scope. Returns [] if missing."""
    path = _manifest_path(worktree_dir, scope)
    if not path.is_file():
        return []
    try:
        payload = json.loads(path.read_text())
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
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2))


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
        rel = str(fpath.relative_to(project_root))
        if classify_file_bucket(rel) == bucket_filter:
            out.append(fpath)
    return out


def _build_manifest_entries(project_root: Path, paths: List[Path]) -> List[Dict[str, Any]]:
    entries: List[Dict[str, Any]] = []
    for fpath in paths:
        try:
            stat = fpath.stat()
        except OSError:
            continue
        rel = str(fpath.relative_to(project_root))
        entries.append({
            "path": rel,
            "mtime": int(stat.st_mtime),
            "size": int(stat.st_size),
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
            rel = str(fpath.relative_to(project_root))
        except ValueError:
            continue
        try:
            stat = fpath.stat()
        except OSError:
            continue
        desc = extract_description(fpath)
        try:
            text = fpath.read_text(errors="replace")[:2000]
        except OSError:
            text = ""
        ids.append(rel)
        documents.append(f"{rel}\n{desc}\n{text}")
        metadatas.append(
            {
                "path": rel,
                "description": desc,
                "file_type": fpath.suffix.lstrip(".") or "unknown",
                "size": int(stat.st_size),
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


# ---------------------------------------------------------------------
# v2 actions: index-files
# ---------------------------------------------------------------------


def action_index_files_v2(
    project_root: str,
    repo_hash: str,
    worktree_hash: str,
    mode: str = "full",
    db_root: Optional[Path] = None,
    scope: str = "files",
) -> dict:
    """Index project files into ChromaDB under the v2 layout."""
    root = Path(project_root).resolve()

    db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
    bucket = "code" if scope == "files" else "docs"

    paths = _scan_files(root, bucket_filter=bucket)
    new_entries = _build_manifest_entries(root, paths)

    with acquire_lock(db_path, exclusive=True):
        client, collection = _make_chroma_collection(
            db_path,
            V2_FILES_CODE_COLLECTION if scope == "files" else V2_FILES_DOCS_COLLECTION,
        )

        if mode == "incremental":
            old_entries = read_manifest(db_path, scope=scope)
            diff = compute_manifest_diff(old_entries, new_entries)
            to_embed = diff["added"] + diff["changed"]
            to_delete = diff["removed"]

            embedded_paths = [root / rel for rel in to_embed]
            count = embed_documents_for_paths(embedded_paths, root, collection)
            _delete_paths_from_collection(collection, to_delete)
        else:
            # full mode
            try:
                existing = collection.get()
                if existing.get("ids"):
                    collection.delete(ids=existing["ids"])
            except Exception:
                pass

            count = embed_documents_for_paths(paths, root, collection)

        write_manifest(db_path, scope=scope, entries=new_entries)

    return {
        "ok": True,
        "scope": scope,
        "indexed": count,
        "total": len(new_entries),
    }


# ---------------------------------------------------------------------
# v2 actions: index-specs
# ---------------------------------------------------------------------


def action_index_specs_v2(
    project_root: str,
    repo_hash: str,
    worktree_hash: str,
    mode: str = "full",
    db_root: Optional[Path] = None,
) -> dict:
    """Index local SPEC directories into ChromaDB under the v2 layout."""
    root = Path(project_root).resolve()
    db_path = resolve_db_path(repo_hash, worktree_hash, "specs", db_root=db_root)

    specs_dir = root / "specs"
    spec_dirs = sorted(specs_dir.glob("SPEC-*")) if specs_dir.is_dir() else []

    new_entries: List[Dict[str, Any]] = []
    spec_records: List[Dict[str, Any]] = []
    for spec_path in spec_dirs:
        metadata_file = spec_path / "metadata.json"
        if not metadata_file.is_file():
            continue
        try:
            meta = json.loads(metadata_file.read_text(errors="replace"))
        except (json.JSONDecodeError, ValueError, OSError):
            continue
        spec_id = str(meta.get("id", ""))
        title = meta.get("title", "")
        status = meta.get("status", "")
        phase = meta.get("phase", "")
        dir_name = spec_path.name

        spec_md = spec_path / "spec.md"
        spec_content = ""
        if spec_md.is_file():
            try:
                spec_content = spec_md.read_text(errors="replace")[:2000]
                stat = spec_md.stat()
                rel = str(spec_md.relative_to(root))
                new_entries.append(
                    {"path": rel, "mtime": int(stat.st_mtime), "size": int(stat.st_size)}
                )
            except OSError:
                pass

        spec_records.append(
            {
                "id": f"spec-{spec_id}",
                "document": f"{title}\n{spec_content}",
                "metadata": {
                    "spec_id": spec_id,
                    "title": title,
                    "status": status,
                    "phase": phase,
                    "dir_name": dir_name,
                },
            }
        )

    new_entries.sort(key=lambda e: e["path"])

    with acquire_lock(db_path, exclusive=True):
        client, collection = _make_chroma_collection(db_path, V2_SPECS_COLLECTION)

        if mode == "full":
            try:
                existing = collection.get()
                if existing.get("ids"):
                    collection.delete(ids=existing["ids"])
            except Exception:
                pass

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

    return {"ok": True, "scope": "specs", "indexed": len(spec_records)}


# ---------------------------------------------------------------------
# v2 actions: index-issues with TTL
# ---------------------------------------------------------------------


def _read_issue_meta(db_path: Path) -> Optional[Dict[str, Any]]:
    meta_file = db_path / META_FILENAME
    if not meta_file.is_file():
        return None
    try:
        return json.loads(meta_file.read_text())
    except (json.JSONDecodeError, OSError):
        return None


def _write_issue_meta(db_path: Path, payload: Dict[str, Any]) -> None:
    db_path.mkdir(parents=True, exist_ok=True)
    (db_path / META_FILENAME).write_text(json.dumps(payload, ensure_ascii=False, indent=2))


def _now_utc() -> datetime.datetime:
    return datetime.datetime.now(datetime.timezone.utc)


def _parse_iso(value: str) -> Optional[datetime.datetime]:
    try:
        return datetime.datetime.fromisoformat(value)
    except ValueError:
        return None


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
                    return {
                        "ok": True,
                        "skipped": True,
                        "scope": "issues",
                        "ttl_remaining_seconds": int(ttl_minutes * 60 - age),
                    }

    with acquire_lock(db_path, exclusive=True):
        try:
            result = subprocess.run(
                [
                    "gh", "issue", "list",
                    "--state", "all",
                    "--limit", "200",
                    "--json", "number,title,body,labels,state,url",
                ],
                cwd=str(Path(project_root).resolve()),
                capture_output=True,
                encoding="utf-8",
                check=True,
            )
            issues = json.loads(result.stdout) if result.stdout else []
        except subprocess.CalledProcessError as exc:
            return {
                "ok": False,
                "error_code": "RUNTIME_ERROR",
                "error": f"gh issue list failed: {(exc.stderr or '').strip()}",
            }
        except (json.JSONDecodeError, ValueError) as exc:
            return {
                "ok": False,
                "error_code": "RUNTIME_ERROR",
                "error": f"Failed to parse gh output: {exc}",
            }

        client, collection = _make_chroma_collection(db_path, V2_ISSUES_COLLECTION)
        try:
            existing = collection.get()
            if existing.get("ids"):
                collection.delete(ids=existing["ids"])
        except Exception:
            pass

        if issues:
            ids: List[str] = []
            documents: List[str] = []
            metadatas: List[Dict[str, Any]] = []
            for issue in issues:
                number = issue.get("number", 0)
                title = issue.get("title", "")
                body = (issue.get("body") or "")[:2000]
                state = issue.get("state", "")
                url = issue.get("url", "")
                labels = [lbl.get("name", "") for lbl in issue.get("labels", [])]
                ids.append(str(number))
                documents.append(f"{title}\n{body}")
                metadatas.append(
                    {
                        "number": number,
                        "title": title,
                        "url": url,
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

        _write_issue_meta(
            db_path,
            {
                "schema_version": INDEX_SCHEMA_VERSION,
                "last_full_refresh": _now_utc().isoformat(),
                "ttl_minutes": ttl_minutes,
            },
        )

    return {"ok": True, "scope": "issues", "indexed": len(issues)}


# ---------------------------------------------------------------------
# v2 actions: search-* with auto-build fallback
# ---------------------------------------------------------------------


def _search_collection_v2(collection, query: str, n_results: int) -> List[Dict[str, Any]]:
    try:
        count = collection.count()
    except Exception:
        return []
    if count == 0:
        return []
    actual_n = min(n_results, count)
    results = collection.query(query_texts=[query], n_results=actual_n)
    items: List[Dict[str, Any]] = []
    if results and results.get("ids") and results["ids"][0]:
        for idx, doc_id in enumerate(results["ids"][0]):
            meta = results["metadatas"][0][idx] if results.get("metadatas") else {}
            distance = results["distances"][0][idx] if results.get("distances") else None
            items.append(
                {
                    "id": doc_id,
                    "metadata": meta,
                    "distance": round(distance, 4) if distance is not None else None,
                }
            )
    return items


def _format_file_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    return [
        {
            "path": (it["metadata"] or {}).get("path", it["id"]),
            "description": (it["metadata"] or {}).get("description", ""),
            "distance": it["distance"],
            "fileType": (it["metadata"] or {}).get("file_type", ""),
            "size": (it["metadata"] or {}).get("size", 0),
        }
        for it in items
    ]


def _format_spec_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    return [
        {
            "spec_id": (it["metadata"] or {}).get("spec_id", it["id"]),
            "title": (it["metadata"] or {}).get("title", ""),
            "status": (it["metadata"] or {}).get("status", ""),
            "phase": (it["metadata"] or {}).get("phase", ""),
            "dir_name": (it["metadata"] or {}).get("dir_name", ""),
            "distance": it["distance"],
        }
        for it in items
    ]


def _format_issue_results(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    formatted = []
    for it in items:
        meta = it["metadata"] or {}
        labels_raw = meta.get("labels", "")
        labels = [lb for lb in labels_raw.split(",") if lb] if labels_raw else []
        formatted.append(
            {
                "number": meta.get("number", it["id"]),
                "title": meta.get("title", ""),
                "url": meta.get("url", ""),
                "state": meta.get("state", ""),
                "labels": labels,
                "distance": it["distance"],
            }
        )
    return formatted


def action_search_v2(
    action: str,
    repo_hash: str,
    worktree_hash: Optional[str],
    project_root: Optional[str],
    query: str,
    n_results: int = 10,
    no_auto_build: bool = False,
    db_root: Optional[Path] = None,
) -> dict:
    """Unified v2 search dispatcher with auto-build fallback."""
    scope_for_action = {
        "search-files": "files",
        "search-files-docs": "files-docs",
        "search-specs": "specs",
        "search-issues": "issues",
    }
    if action not in scope_for_action:
        return {"ok": False, "error_code": "BAD_ARGS", "error": f"unknown action {action}"}
    scope = scope_for_action[action]

    db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
    chroma_sqlite = db_path / "chroma.sqlite3"

    if not chroma_sqlite.exists():
        if no_auto_build:
            return {
                "ok": False,
                "error_code": "INDEX_MISSING",
                "error": f"index not found at {db_path}",
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
        collection_name = {
            "files": V2_FILES_CODE_COLLECTION,
            "files-docs": V2_FILES_DOCS_COLLECTION,
            "specs": V2_SPECS_COLLECTION,
            "issues": V2_ISSUES_COLLECTION,
        }[scope]
        client, collection = _make_chroma_collection(db_path, collection_name)
        items = _search_collection_v2(collection, query, n_results)

    if scope in ("files", "files-docs"):
        return {"ok": True, "results": _format_file_results(items)}
    if scope == "specs":
        return {"ok": True, "specResults": _format_spec_results(items)}
    return {"ok": True, "issueResults": _format_issue_results(items)}


# ---------------------------------------------------------------------
# v2 status
# ---------------------------------------------------------------------


def action_status_v2(
    repo_hash: str,
    worktree_hash: Optional[str],
    db_root: Optional[Path] = None,
) -> dict:
    issues_path = resolve_db_path(repo_hash, None, "issues", db_root=db_root)
    issues_status: Dict[str, Any] = {
        "exists": (issues_path / "chroma.sqlite3").exists()
        or (issues_path / META_FILENAME).is_file(),
    }
    meta = _read_issue_meta(issues_path)
    if meta:
        issues_status.update(
            {
                "last_full_refresh": meta.get("last_full_refresh"),
                "ttl_minutes": meta.get("ttl_minutes", ISSUE_TTL_MINUTES_DEFAULT),
            }
        )
        last = _parse_iso(meta.get("last_full_refresh", "")) if meta.get("last_full_refresh") else None
        if last is not None:
            age = (_now_utc() - last).total_seconds()
            ttl_secs = meta.get("ttl_minutes", ISSUE_TTL_MINUTES_DEFAULT) * 60
            issues_status["ttl_remaining_seconds"] = max(0, int(ttl_secs - age))

    out: Dict[str, Any] = {"issues": issues_status}
    if worktree_hash:
        for scope in ("specs", "files", "files-docs"):
            db_path = resolve_db_path(repo_hash, worktree_hash, scope, db_root=db_root)
            out[scope] = {"exists": (db_path / "chroma.sqlite3").exists()}

    return {"ok": True, "status": out}


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
            "search-files",
            "search-files-docs",
            "index",
            "search",
            "status",
            "index-issues",
            "search-issues",
            "index-specs",
            "search-specs",
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
        choices=["", "issues", "specs", "files", "files-docs"],
    )
    parser.add_argument("--mode", default="full", choices=["full", "incremental"])
    parser.add_argument("--no-auto-build", dest="no_auto_build", action="store_true")
    parser.add_argument("--respect-ttl", dest="respect_ttl", action="store_true")
    return parser.parse_args()


def _dispatch_v2(action: str, args: argparse.Namespace) -> int:
    """Phase 8 v2 dispatcher."""
    repo_hash = args.repo_hash
    worktree_hash = args.worktree_hash or None

    try:
        if action == "status":
            emit(action_status_v2(repo_hash, worktree_hash))
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
                )
            )
            return 0

        if action in ("search-files", "search-files-docs", "search-specs", "search-issues"):
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
