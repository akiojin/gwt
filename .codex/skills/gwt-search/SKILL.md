---
name: gwt-search
description: "Mandatory preflight before gwt-discussion, gwt-register-issue, and gwt-fix-issue. Use proactively before creating any SPEC or Issue owner or before reusing an existing one. Searches local SPECs, GitHub Issues, and project files via ChromaDB. Triggers: 'search', 'find related', 'check duplicates'."
---

# Unified Search

gwt maintains ChromaDB vector search indexes for three scopes (Phase 8 layout):

| Scope | Content | Lifecycle |
|-------|---------|-----------|
| SPECs | GitHub Issue cache (`~/.gwt/cache/issues/`) | Populated by `gwt issue spec pull` or TUI startup sync |
| Issues | GitHub Issues (all states) | TUI startup async refresh (TTL 15 min) + runner auto-build on first search |
| Files | Project implementation files (excludes skill assets, SPEC trees, snapshots) | Watcher (TUI) + runner auto-build on first search |

All vector data is stored under `~/.gwt/index/<repo-hash>/...`. Issues are repo-scoped and shared across worktrees; SPECs and Files are worktree-scoped under `worktrees/<worktree-hash>/`. The legacy `$WORKTREE/.gwt/index/` location is no longer used and is deleted automatically by the TUI on startup.

When invoked outside the gwt TUI, the runner falls back to a synchronous mtime+size diff per call: results are always correct, just slower than the TUI watcher path.

## Quick reference

```text
gwt-search "query"              # search all three scopes
gwt-search --specs "query"      # SPECs only
gwt-search --issues "query"     # GitHub Issues only
gwt-search --files "query"      # implementation files only
```

## Filter options

| Flag | Scope | Action flag |
|------|-------|------------|
| (none) | All three | Run all three searches |
| `--specs` | SPECs only | `search-specs` |
| `--issues` | Issues only | `search-issues` |
| `--files` | Files only | `search-files` |

## Environment

When the gwt TUI launches an agent pane, the following env vars are exported automatically:

- `GWT_PROJECT_ROOT` — absolute path of the active worktree
- `GWT_REPO_HASH` — SHA256[:16] of the normalized origin URL
- `GWT_WORKTREE_HASH` — SHA256[:16] of the canonicalized worktree absolute path

If you launch outside the TUI, recompute them:

```bash
GWT_PROJECT_ROOT="$(pwd)"
GWT_REPO_HASH=$(git remote get-url origin 2>/dev/null \
  | sed -E 's#^git@([^:]+):#https://\1/#; s#\.git$##; s#^https?://##' \
  | tr 'A-Z' 'a-z' | sha256sum | cut -c1-16)
GWT_WORKTREE_HASH=$(printf '%s' "$(cd "$GWT_PROJECT_ROOT" && pwd -P)" | sha256sum | cut -c1-16)
```

## Search commands

```bash
PYTHON=~/.gwt/runtime/chroma-venv/bin/python3
RUNNER=~/.gwt/runtime/chroma_index_runner.py
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe`.

### Search SPECs

```bash
$PYTHON $RUNNER \
  --action search-specs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

### Search GitHub Issues

```bash
$PYTHON $RUNNER \
  --action search-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

### Search project files (code)

```bash
$PYTHON $RUNNER \
  --action search-files \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

`search-files` is implementation-focused: it excludes embedded skill assets (`.claude/`, `.codex/`), local/archived SPEC trees, local task logs, and snapshot files so code search is not dominated by docs noise.

### Search project docs

```bash
$PYTHON $RUNNER \
  --action search-files-docs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

### Search all scopes (default)

Run all four search commands above and merge results by scope.

## Auto-build fallback

When the target index does not exist, the runner builds it inline (full mode) and then performs the search. Progress is emitted as NDJSON on stderr:

```text
{"phase":"indexing","scope":"files","done":0,"total":0}
{"phase":"complete","scope":"files","total":850}
```

Pass `--no-auto-build` to disable this behavior; in that case the runner returns:

```json
{"ok": false, "error_code": "INDEX_MISSING", "error": "index not found at ..."}
```

## Index update commands

These are run automatically by the TUI watcher (or by the runner's auto-build fallback). Run manually only when forcing a full rebuild.

### Update SPEC index (force full)

```bash
$PYTHON $RUNNER \
  --action index-specs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full
```

### Update Issues index (force, ignore TTL)

```bash
$PYTHON $RUNNER \
  --action index-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT"
```

Pass `--respect-ttl` to skip if the previous refresh is younger than 15 minutes.

### Update file index (force full)

```bash
$PYTHON $RUNNER \
  --action index-files \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full \
  --scope files
```

For the docs collection, repeat with `--scope files-docs`.

## Output formats

### SPEC results

```json
{"ok": true, "specResults": [
  {"spec_id": "10", "title": "Project workspace", "status": "in-progress", "phase": "Implementation", "dir_name": "SPEC-10", "distance": 0.08}
]}
```

### Issue results

```json
{"ok": true, "issueResults": [
  {"number": 42, "title": "Add vector search for Issues", "url": "https://github.com/...", "state": "open", "labels": ["enhancement"], "distance": 0.08}
]}
```

### File results

```json
{"ok": true, "results": [
  {"path": "src/git/issue.rs", "description": "GitHub Issue commands", "distance": 0.12}
]}
```

## Interpreting results

- Lower distance values indicate higher relevance (0.0 = exact match)
- Uses semantic similarity, not just keyword matching
- Results are ranked by distance within each scope
- The embedding model is `intfloat/multilingual-e5-base` (multilingual; handles Japanese)

## When to use

### Mandatory preflight

This skill is a **mandatory preflight step** before:

- `gwt-discussion`
- `gwt-register-issue`
- `gwt-fix-issue`
- any visible workflow that must decide an existing SPEC/Issue owner

Run at least 2-3 semantic queries derived from the request before creating any new SPEC or Issue.

### General use cases

- **Spec integration**: find the canonical spec before creating or updating
- **Issue lookup**: find existing GitHub Issues before creating new ones
- **Task start**: search for specs, issues, and files related to the assigned feature
- **Bug investigation**: find issues and files that might relate to the bug
- **Duplicate check**: verify no existing spec or issue covers the same scope
- **Architecture understanding**: discover how features are specified and implemented
- **Feature addition**: locate existing similar implementations across all scopes

### Trigger phrases

- "search specs / issues / files"
- "find related specs / issues / files"
- "check for duplicates"
- "which spec / issue handles X"
- "既存仕様を探して"
- "関連 Issue を探して"
- "どの SPEC に統合するべきか"
- "重複する SPEC はないか確認して"
- "この機能の仕様は？"

## Suggested query patterns

Use 2-3 queries with different angles for thorough coverage:

- **Subsystem + purpose**: `project index issue search spec`
- **User-facing problem + architecture term**: `chroma persisted db recovery project index`
- **Workflow + discoverability**: `LLM should use search before spec creation`
- **Japanese keywords**: `TUI ナビゲーション キーバインド`
- **Domain concept**: `worktree management branch isolation`

## Minimum search workflow

1. Run searches with 2-3 semantic queries derived from the request
2. The runner auto-builds any missing index on the first call
3. Pick the canonical existing spec or issue if found
4. Only fall back to creating a new spec or issue when no suitable canonical match exists
