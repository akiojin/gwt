---
name: gwt-search
description: "Mandatory preflight before gwt-discussion, gwt-register-issue, and gwt-fix-issue. Use proactively before creating any SPEC or Issue owner or before reusing an existing one. Searches SPEC Issues, GitHub Issues, project files, and post-mortem memory via ChromaDB. Triggers: 'search', 'find related', 'check duplicates', '過去 memory を引いて'."
---

# Unified Search

gwt maintains ChromaDB vector search indexes for four scopes (Phase 8 layout
plus SPEC-2805 Memory):

| Scope | Content | Lifecycle |
|-------|---------|-----------|
| SPECs | GitHub Issue cache (`~/.gwt/cache/issues/`) | Populated by `gwtd issue spec pull` or gwt GUI startup sync |
| Issues | GitHub Issues (all states) | gwt GUI startup async refresh (TTL 15 min) + runner auto-build on first search |
| Files | Project implementation files (excludes skill assets, SPEC trees, snapshots) | Per-worktree watcher (gwt GUI) + runner auto-build on first search |
| Memory | Post-mortem entries in `.gwt/work/memory.md` | Pinpoint allowlist watcher on `.gwt/work/memory.md` + runner auto-build on first search |

All vector data is stored under `~/.gwt/index/<repo-hash>/...`. Issues,
SPECs, and Memory are repo-scoped and shared across worktrees; Files (code
+ docs) is worktree-scoped under `worktrees/<worktree-hash>/`. The legacy
`$WORKTREE/.gwt/index/` location is no longer used and is deleted
automatically by the gwt GUI on startup.

When invoked outside the gwt GUI app, the runner falls back to a
synchronous mtime+size diff per call: results are always correct, just
slower than the GUI watcher path.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Quick reference

```text
gwt-search "query"              # search all four scopes (default merge)
gwt-search --specs "query"      # SPECs only
gwt-search --issues "query"     # GitHub Issues only
gwt-search --files "query"      # implementation files only
gwt-search --memory "query"    # post-mortem memory only
```

## Filter options

| Flag | Scope | Action flag |
|------|-------|------------|
| (none) | All four | Run all four searches |
| `--specs` | SPECs only | `search-specs` |
| `--issues` | Issues only | `search-issues` |
| `--files` | Files only | `search-files` |
| `--memory` | Memory only | `search-memory` |

## Match modes

Use the default semantic mode for broad discovery. Use `--match-mode all_terms`
when the user or task needs FAQ-style precision and every whitespace-separated
term or quoted phrase must be present in a strict result.

Examples:

```text
gwt-search --match-mode all_terms "Workspace 置き換え"
gwt-search --match-mode all_terms "\"Project State\" migration"
```

In `all_terms` mode, strict results must satisfy every required term. Semantic
suggestions may still be returned separately, but they must not be treated as
strict matches.

## Environment

When the gwt GUI app (WebView built with `wry + tao + axum WebSocket` and
`xterm.js`) launches an agent pane, the following env vars are exported
automatically:

- `GWT_PROJECT_ROOT` — absolute path of the active worktree
- `GWT_REPO_HASH` — SHA256[:16] of the normalized origin URL
- `GWT_WORKTREE_HASH` — SHA256[:16] of the canonicalized worktree absolute path

The hashes are an optimization, not a requirement: when `GWT_REPO_HASH` /
`GWT_WORKTREE_HASH` are unset or passed empty (e.g. when the launch environment
did not export them), the runner derives them from `--project-root`
automatically (Issue #2933). A search therefore needs only `--project-root`,
and works in any shell on any platform — no manual hash recomputation, and no
dependency on `sha256sum` (which is absent on stock macOS).

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

### Search Memory

```bash
$PYTHON $RUNNER \
  --action search-memory \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

`search-memory` reads from the repo-scoped memory index built from
`<project_root>/.gwt/work/memory.md`. `--worktree-hash` is accepted but ignored
for this scope.

### Search all scopes (default)

Run all five search commands above (SPECs, Issues, Files-code, Files-docs,
Memory) and merge results by scope.

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

## Empty corpus is a tooling failure, not "no results"

`search-specs` and `search-issues` build their corpus from the GitHub Issue
cache (`~/.gwt/cache/issues/<repo-hash>/`). When that cache is empty or
unpopulated for the repo-hash, an auto-build search would index zero documents.
Instead of silently returning `ok: true` with an empty list — which reads as
"no existing SPEC/Issue owner" and causes duplicate creation — the runner
returns a diagnostic:

```json
{"ok": false, "error_code": "EMPTY_CORPUS", "scope": "specs",
 "issue_cache_dir": "~/.gwt/cache/issues/<repo-hash>",
 "issue_cache_populated": false,
 "error": "specs search corpus is empty: ... Refresh the cache ... and retry ..."}
```

When you see `EMPTY_CORPUS`, **do not conclude that no owner exists.** Refresh
the issue cache (open the project in the gwt GUI to sync, or run a `gwtd issue`
sync) and retry the search. Only an `ok: true` result with an empty list from a
*populated* cache means the repository genuinely has no matching SPEC/Issue.

## Index update commands

These are run automatically by the gwt GUI watcher (or by the runner's
auto-build fallback). Run manually only when forcing a full rebuild.

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

### Update memory index (force full)

```bash
$PYTHON $RUNNER \
  --action index-memory \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full
```

Memory is repo-scoped; `--worktree-hash` is accepted but ignored.

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

### Memory results

```json
{"ok": true, "memoryResults": [
  {"date": "2026-05-20", "title": "gwtd issue spec create -f は section マーカーを付けない", "heading": "## 2026-05-20 — gwtd issue spec create -f は section マーカーを付けない", "chunk_idx": 0, "distance": 0.12}
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
- **Memory lookup**: before fixing a bug, check whether a prior `.gwt/work/memory.md` entry already records the prevention strategy
- **Task start**: search for specs, issues, files, and memory related to the assigned feature
- **Bug investigation**: find issues, files, and memory that might relate to the bug
- **Duplicate check**: verify no existing spec, issue, or memory covers the same scope
- **Architecture understanding**: discover how features are specified, implemented, and previously failed
- **Feature addition**: locate existing similar implementations and recurring failure modes across all scopes

### Trigger phrases

- "search specs / issues / files / memory"
- "find related specs / issues / files / memory"
- "check for duplicates"
- "which spec / issue handles X"
- "has this regression been recorded?"
- "既存仕様を探して"
- "関連 Issue を探して"
- "どの SPEC に統合するべきか"
- "重複する SPEC はないか確認して"
- "この機能の仕様は？"
- "過去 memory を引いて"
- "同じ失敗があるか確認して"

## Suggested query patterns

Use 2-3 queries with different angles for thorough coverage:

- **Subsystem + purpose**: `project index issue search spec`
- **User-facing problem + architecture term**: `chroma persisted db recovery project index`
- **Workflow + discoverability**: `LLM should use search before spec creation`
- **Japanese keywords**: `ターミナル ナビゲーション キーバインド`
- **Domain concept**: `worktree management branch isolation`
- **Past failure**: `watcher debounce silent failure`, `spec section マーカー罠`

## Minimum search workflow

1. Run searches with 2-3 semantic queries derived from the request
2. The runner auto-builds any missing index on the first call
3. Pick the canonical existing spec, issue, or memory if found
4. Only fall back to creating a new spec or issue when no suitable canonical match exists
