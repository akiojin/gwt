---
name: gwt-search
description: "Mandatory preflight before gwt-spec-design and gwt-issue. Use proactively before creating any SPEC or Issue to prevent duplicates. Searches local SPECs, GitHub Issues, and project files via ChromaDB. Triggers: 'search', 'find related', 'check duplicates'."
---

# Unified Search

gwt maintains ChromaDB vector search indexes for three scopes:

| Scope | Content | Index maintenance |
|-------|---------|-------------------|
| SPECs | Local SPEC files (`specs/SPEC-{N}/`) | Automatic (file system watcher) |
| Issues | GitHub Issues (all states) | Manual (`index-issues` action or GUI button) |
| Files | Project source files | Automatic (file system watcher) |

## Quick reference

```text
gwt-search "query"              # search all three scopes
gwt-search --specs "query"      # SPECs only
gwt-search --issues "query"     # GitHub Issues only
gwt-search --files "query"      # project source files only
```

## Filter options

| Flag | Scope | Action flag | Notes |
|------|-------|------------|-------|
| (none) | All three | Run all three searches | Default behavior |
| `--specs` | SPECs only | `search-specs` | Local `specs/SPEC-{N}/` directories |
| `--issues` | Issues only | `search-issues` | GitHub Issues via ChromaDB index |
| `--files` | Files only | `search` | Project source files |

## Search commands

All commands use the same runner script and database path:

```bash
PYTHON=~/.gwt/runtime/chroma-venv/bin/python3
RUNNER=~/.gwt/runtime/chroma_index_runner.py
DB_PATH="$GWT_PROJECT_ROOT/.gwt/index"
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe` as the Python executable.

### Search SPECs

```bash
$PYTHON $RUNNER \
  --action search-specs \
  --db-path "$DB_PATH" \
  --query "your search query" \
  --n-results 10
```

### Search GitHub Issues

```bash
$PYTHON $RUNNER \
  --action search-issues \
  --db-path "$DB_PATH" \
  --query "your search query" \
  --n-results 10
```

### Search project files

```bash
$PYTHON $RUNNER \
  --action search \
  --db-path "$DB_PATH" \
  --query "your search query" \
  --n-results 10
```

### Search all scopes (default)

Run all three search commands above and merge results by scope.

## Index update commands

### Update SPEC index (normally automatic)

```bash
$PYTHON $RUNNER \
  --action index-specs \
  --project-root "$GWT_PROJECT_ROOT" \
  --db-path "$DB_PATH"
```

### Update Issues index (manual — required before first Issues search)

```bash
$PYTHON $RUNNER \
  --action index-issues \
  --db-path "$DB_PATH"
```

### Update file index (normally automatic)

```bash
$PYTHON $RUNNER \
  --action index-files \
  --project-root "$GWT_PROJECT_ROOT" \
  --db-path "$DB_PATH"
```

## Output formats

### SPEC results

```json
{"ok": true, "specResults": [
  {"spec_id": "1579", "title": "gwt-spec system", "status": "open", "phase": "ready", "dir_name": "SPEC-1579", "distance": 0.08}
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
  {"path": "src/git/issue.rs", "description": "GitHub Issue commands", "distance": 0.12},
  {"path": "src/lib/components/IssuePanel.svelte", "description": "Issue list panel", "distance": 0.25}
]}
```

## Interpreting results

- Lower distance values indicate higher relevance (0.0 = exact match)
- Uses semantic similarity, not just keyword matching
- Results are ranked by distance within each scope

## When to use

### Mandatory preflight

This skill is a **mandatory preflight step** before:

- `gwt-spec-design` (spec brainstorm, register, clarify, ops)
- `gwt-spec-register` / `gwt-spec-ops`
- `gwt-issue-register` / `gwt-issue-resolve`

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

1. For Issues scope: update the index first with `index-issues` (SPECs and files are auto-indexed)
2. Run searches with 2-3 semantic queries derived from the request
3. Pick the canonical existing spec or issue if found
4. Only fall back to creating a new spec or issue when no suitable canonical match exists

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)
