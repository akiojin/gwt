---
name: gwt-project-search
description: "Semantic search over project source files using vector embeddings. Use when the user asks to search project files, find related implementation files, or locate source files for a feature, bug, or concept."
---

# Project Search

gwt maintains a vector search index of project implementation files using ChromaDB embeddings (model: `intfloat/multilingual-e5-base`). The index is stored at `~/.gwt/index/<repo-hash>/worktrees/<worktree-hash>/files/` (with a sibling `files-docs/` collection for documentation). The gwt TUI keeps a per-Worktree filesystem watcher running so changes flow into the index automatically. When invoked outside the TUI, the runner auto-builds the index on the first call.

## Environment

When the gwt TUI launches an agent pane, the following env vars are exported automatically:

- `GWT_PROJECT_ROOT` — absolute path of the active worktree
- `GWT_REPO_HASH` — SHA256[:16] of the normalized origin URL
- `GWT_WORKTREE_HASH` — SHA256[:16] of the canonicalized worktree absolute path

## File search command (code)

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-files \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe` as the Python executable.

## Project docs search

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-files-docs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

## File search output format

```json
{"ok": true, "results": [
  {"path": "src/git/issue.rs", "description": "GitHub Issue commands", "distance": 0.12},
  {"path": "src/lib/components/IssuePanel.svelte", "description": "Issue list panel", "distance": 0.25}
]}
```

## When to use

- Task start: search for project files related to the assigned feature
- Bug investigation: find implementation files that might relate to the bug
- Feature addition: locate existing similar implementations in the project
- Architecture understanding: discover how project components are organized

## Notes

- The TUI watcher (2 s debounce, 100-file batch) keeps the index live; non-TUI sessions get an mtime+size diff per call
- The runner auto-builds the index when missing (use `--no-auto-build` to suppress)
- `search-files` is implementation-focused and excludes embedded skill assets, local/archived SPEC trees, local task logs, and snapshot files
- Project docs are indexed separately and can be searched with `search-files-docs`
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- Canonical standalone skill name: `gwt-project-search`
- Internal runner actions remain `search-files` / `index-files`
- For SPEC search, use `gwt-spec-search` instead
- For Issue search, use `gwt-issue-search` instead
