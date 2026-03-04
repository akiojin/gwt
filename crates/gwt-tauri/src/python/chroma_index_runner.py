#!/usr/bin/env python3
"""ChromaDB project index helper for gwt.

This helper is executed by Rust backend commands and returns JSON on stdout.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import List, Optional


def emit(payload: dict) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=True))
    sys.stdout.flush()


# ---------------------------------------------------------------------------
# .gitignore matching (simplified)
# ---------------------------------------------------------------------------

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
        return None  # skip negation patterns for simplicity

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


def collect_files(project_root: Path) -> List[Path]:
    """Recursively collect project files, respecting .gitignore."""
    patterns = load_gitignore_patterns(project_root)
    compiled = [p for p in (_pattern_to_regex(pat) for pat in patterns) if p is not None]
    result = []
    for root, dirs, files in os.walk(project_root):
        root_path = Path(root)
        rel_root = root_path.relative_to(project_root)
        rel_root_str = str(rel_root) if str(rel_root) != "." else ""

        # Prune ignored directories
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


# ---------------------------------------------------------------------------
# Description extraction
# ---------------------------------------------------------------------------

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

    # Rust: //! module doc comment
    if suffix == ".rs":
        for line in lines[:20]:
            stripped = line.strip()
            if stripped.startswith("//!"):
                text = stripped[3:].strip()
                if text:
                    return text
        return file_path.name

    # TypeScript / JavaScript: first // or /** */ comment
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
                # Check next line
                for next_line in lines[1:10]:
                    next_stripped = next_line.strip().lstrip("* ").rstrip("*/").strip()
                    if next_stripped:
                        return next_stripped
                break
        return file_path.name

    # Svelte: <script> section first comment
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

    # Markdown: # title
    if suffix in (".md", ".mdx"):
        for line in lines[:10]:
            stripped = line.strip()
            if stripped.startswith("# "):
                return stripped[2:].strip()
        return file_path.name

    # TOML: description field
    if suffix == ".toml" or name == "cargo.toml":
        for line in lines[:50]:
            stripped = line.strip()
            if stripped.startswith("description"):
                match = re.match(r'description\s*=\s*"(.+?)"', stripped)
                if match:
                    return match.group(1)
        return file_path.name

    # JSON/JSONC: description field
    if suffix in (".json", ".jsonc") and "package" in name:
        try:
            data = json.loads(content)
            if isinstance(data, dict) and "description" in data:
                return str(data["description"])
        except (json.JSONDecodeError, ValueError):
            pass
        return file_path.name

    # Python: module docstring or first comment
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

    # YAML: first comment
    if suffix in (".yml", ".yaml"):
        for line in lines[:10]:
            stripped = line.strip()
            if stripped.startswith("#"):
                text = stripped[1:].strip()
                if text:
                    return text
        return file_path.name

    return file_path.name


# ---------------------------------------------------------------------------
# Actions
# ---------------------------------------------------------------------------

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
    collection = client.get_or_create_collection(
        name="files",
        metadata={"hnsw:space": "cosine"},
    )

    files = collect_files(root)
    current_ids = {str(f.relative_to(root)) for f in files}

    # Batch upsert
    batch_size = 100
    total_indexed = 0

    if files:
        for i in range(0, len(files), batch_size):
            batch = files[i : i + batch_size]
            ids = []
            documents = []
            metadatas = []

            for fpath in batch:
                rel = str(fpath.relative_to(root))
                desc = extract_description(fpath)
                try:
                    size = fpath.stat().st_size
                except OSError:
                    size = 0

                ids.append(rel)
                documents.append(f"{rel}: {desc}")
                metadatas.append({
                    "path": rel,
                    "description": desc,
                    "file_type": fpath.suffix.lstrip(".") or "unknown",
                    "size": size,
                })

            collection.upsert(ids=ids, documents=documents, metadatas=metadatas)
            total_indexed += len(batch)

    # Remove stale entries (including the empty-file-set case)
    try:
        existing = collection.get()
        stale = [eid for eid in existing["ids"] if eid not in current_ids]
        if stale:
            collection.delete(ids=stale)
    except Exception as exc:
        # Non-critical: keep indexing successful but preserve diagnostics.
        print(f"Warning: stale entry cleanup failed: {exc}", file=sys.stderr)

    elapsed = int((time.monotonic() - start) * 1000)
    return {
        "ok": True,
        "filesIndexed": total_indexed,
        "durationMs": elapsed,
    }


def action_search(db_path: str, query: str, n_results: int = 10) -> dict:
    """Search the project index."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
        return {"ok": False, "error": f"Index not found at {db}"}

    client = chromadb.PersistentClient(path=str(db))
    try:
        collection = client.get_collection("files")
    except Exception:
        return {"ok": False, "error": "Collection 'files' not found. Run index first."}

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
    """Index GitHub Issues (gwt-spec label) into ChromaDB collection 'issues'."""
    import chromadb  # type: ignore

    root = Path(project_root).resolve()
    db = Path(db_path).resolve()
    db.mkdir(parents=True, exist_ok=True)

    start = time.monotonic()

    try:
        result = subprocess.run(
            [
                "gh", "issue", "list",
                "--label", "gwt-spec",
                "--state", "all",
                "--limit", "200",
                "--json", "number,title,body,labels,state,url",
            ],
            cwd=str(root),
            capture_output=True,
            text=True,
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

    # Clear existing entries before re-indexing
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


def action_status(db_path: str) -> dict:
    """Get index status."""
    import chromadb  # type: ignore

    db = Path(db_path).resolve()
    if not db.is_dir():
        return {"ok": True, "indexed": False, "totalFiles": 0}

    client = chromadb.PersistentClient(path=str(db))
    try:
        collection = client.get_collection("files")
        total = collection.count()
    except Exception:
        return {"ok": True, "indexed": False, "totalFiles": 0}

    # Estimate DB size
    db_size = sum(f.stat().st_size for f in db.rglob("*") if f.is_file())

    return {
        "ok": True,
        "indexed": True,
        "totalFiles": total,
        "dbSizeBytes": db_size,
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="gwt ChromaDB index helper")
    parser.add_argument(
        "--action",
        required=True,
        choices=["probe", "index", "search", "status", "index-issues", "search-issues"],
    )
    parser.add_argument("--project-root", default="")
    parser.add_argument("--db-path", default="")
    parser.add_argument("--query", default="")
    parser.add_argument("--n-results", type=int, default=10)
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    try:
        if args.action == "probe":
            emit(action_probe())
            return 0

        if args.action == "index":
            if not args.project_root:
                emit({"ok": False, "error": "--project-root is required for index"})
                return 2
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for index"})
                return 2
            emit(action_index(args.project_root, args.db_path))
            return 0

        if args.action == "search":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for search"})
                return 2
            if not args.query:
                emit({"ok": False, "error": "--query is required for search"})
                return 2
            emit(action_search(args.db_path, args.query, args.n_results))
            return 0

        if args.action == "status":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for status"})
                return 2
            emit(action_status(args.db_path))
            return 0

        if args.action == "index-issues":
            if not args.project_root:
                emit({"ok": False, "error": "--project-root is required for index-issues"})
                return 2
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for index-issues"})
                return 2
            emit(action_index_issues(args.project_root, args.db_path))
            return 0

        if args.action == "search-issues":
            if not args.db_path:
                emit({"ok": False, "error": "--db-path is required for search-issues"})
                return 2
            if not args.query:
                emit({"ok": False, "error": "--query is required for search-issues"})
                return 2
            emit(action_search_issues(args.db_path, args.query, args.n_results))
            return 0

        emit({"ok": False, "error": f"Unsupported action: {args.action}"})
        return 2

    except Exception as exc:
        emit({
            "ok": False,
            "error": str(exc),
            "exceptionType": type(exc).__name__,
        })
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
