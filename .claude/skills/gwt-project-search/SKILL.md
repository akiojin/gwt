---
name: gwt-project-search
description: "Semantic search over project source files using vector embeddings. Use when the user asks to search project files, find related implementation files, or locate source files for a feature, bug, or concept."
---

# Project Search

gwt maintains a vector search index of project implementation files using ChromaDB embeddings (model: `intfloat/multilingual-e5-base`). The index is stored at `~/.gwt/index/<repo-hash>/worktrees/<worktree-hash>/files/` (with a sibling `files-docs/` collection for documentation). The gwt GUI app (WebView built with `wry + tao + axum WebSocket` and `xterm.js`) keeps a per-Worktree filesystem watcher running so changes flow into the index automatically. Outside the gwt app, a missing index is auto-built on the first search.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## File search command (code)

The `search` JSON operation is the canonical gwtd entry point (SPEC-1942
US-15). Run it from inside the target worktree; the repo and worktree are
resolved from the current directory.

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"your search query","scopes":["files"],"n_results":10}}
JSON
```

## Project docs search

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"your search query","scopes":["files_docs"],"n_results":10}}
JSON
```

To force a full re-index (normally handled by the watcher / auto-build):

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"index.rebuild","params":{"scope":"files"}}
JSON
```

## Output format

```json
{"ok": true, "query": "...", "results": [
  {"scope": "files", "title": "src/git/issue.rs", "subtitle": "GitHub Issue commands", "preview": "", "distance": 0.12, "target": {"kind": "file", "path": "src/git/issue.rs"}}
], "suggestions": []}
```

## When to use

- Task start: search for project files related to the assigned feature
- Bug investigation: find implementation files that might relate to the bug
- Feature addition: locate existing similar implementations in the project
- Architecture understanding: discover how project components are organized

## Notes

- The gwt GUI watcher (2 s debounce, 100-file batch) keeps the index live; non-GUI invocations get an mtime+size diff per call
- A missing index is auto-built on the first search
- `params.scopes:["files"]` is implementation-focused and excludes embedded skill assets, local/archived SPEC trees, local task logs, and snapshot files
- Project docs are indexed separately and searched with `params.scopes:["files_docs"]`
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- Canonical standalone skill name: `gwt-project-search`
- For SPEC search, use `gwt-spec-search` instead
- For Issue search, use `gwt-issue-search` instead

## Fallback: direct runner invocation (older binaries only)

Only when the `search` JSON operation is unavailable in an older gwtd binary,
call the Python runner directly with runner action `search-files` (code) or
`search-files-docs` (docs):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-files \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe`. The hashes
are an optimization, not a requirement: when `GWT_REPO_HASH` and
`GWT_WORKTREE_HASH` are unset or passed empty, the runner derives them from
`--project-root` automatically (Issue #2933). The fallback returns the legacy
`{"ok": true, "results": [...]}` shape, and the internal runner actions remain
`search-files` / `index-files`.
