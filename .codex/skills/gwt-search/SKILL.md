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
| Issues | GitHub Issues (all states) | gwt GUI startup async refresh (TTL 15 min) + auto-build on first search |
| Files | Project implementation files (excludes skill assets, SPEC trees, snapshots) | Per-worktree watcher (gwt GUI) + auto-build on first search |
| Memory | Post-mortem entries in `.gwt/work/memory.md` | Pinpoint allowlist watcher on `.gwt/work/memory.md` + auto-build on first search |

All vector data is stored under `~/.gwt/index/<repo-hash>/...`. Issues,
SPECs, and Memory are repo-scoped and shared across worktrees; Files
(code + docs) is worktree-scoped under `worktrees/<worktree-hash>/`. The legacy
`$WORKTREE/.gwt/index/` location is no longer used and is deleted
automatically by the gwt GUI on startup.

When invoked outside the gwt GUI app, the search falls back to a
synchronous mtime+size diff per call: results are always correct, just
slower than the GUI watcher path.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Quick reference

`gwt-search` is a skill, not a PATH executable. Never resolve it with
`command -v` or `Get-Command` — the lookup finds nothing by design, and
an empty lookup does not mean the tooling is missing. The executable
command is the `search` family of the resolved `GWT_BIN` (SPEC-1942
US-15). Run it from inside the target worktree; the repo is resolved from
the current directory.

```bash
"$GWT_BIN" search "query" --json               # search all scopes (default merge)
"$GWT_BIN" search --specs "query" --json       # SPECs only
"$GWT_BIN" search --issues "query" --json      # GitHub Issues only
"$GWT_BIN" search --files "query" --json       # implementation files only
"$GWT_BIN" search --files-docs "query" --json  # project docs only
"$GWT_BIN" search --memory "query" --json      # post-mortem memory only
```

## Filter options

| Flag | Scope |
|------|-------|
| (none) | All scopes merged (same default as the GUI search window) |
| `--specs` | SPEC Issues only |
| `--issues` | GitHub Issues only |
| `--files` | Implementation files only |
| `--files-docs` | Project docs only |
| `--memory` | Post-mortem memory only |
| `--board` | Coordination Board entries only |
| `--discussions` | Git-managed discussion notes only |

Scope flags are repeatable and merge: `--issues --specs` searches both.
Additional options: `--n-results <n>` limits the result count, `--json`
emits machine-readable JSON (recommended for agents), and
`--match-mode semantic|all_terms` selects the match mode.

## Match modes

Use the default semantic mode for broad discovery. Use `--match-mode all_terms`
when the user or task needs FAQ-style precision and every whitespace-separated
term or quoted phrase must be present in a strict result.

Examples:

```bash
"$GWT_BIN" search --match-mode all_terms "Workspace 置き換え" --json
"$GWT_BIN" search --match-mode all_terms "\"Project State\" migration" --json
```

In `all_terms` mode, strict results must satisfy every required term. Semantic
suggestions may still be returned separately (in the `suggestions` array), but
they must not be treated as strict matches.

## Output format

```json
{"ok": true, "query": "...", "results": [
  {"scope": "issues", "title": "#42 Add vector search for Issues", "subtitle": "open", "preview": "enhancement", "distance": 0.08, "target": {"kind": "issue", "number": 42}},
  {"scope": "specs", "title": "SPEC-1939: Semantic search platform", "subtitle": "open · phase/review", "preview": "...", "distance": 0.09, "target": {"kind": "spec", "spec_id": 1939}},
  {"scope": "files", "title": "src/git/issue.rs", "subtitle": "GitHub Issue commands", "preview": "", "distance": 0.12, "target": {"kind": "file", "path": "src/git/issue.rs"}}
], "suggestions": []}
```

- `target` is a kind-tagged locator (`issue`, `spec`, `memory`, `discussion`,
  `board`, `file`) for follow-up reads (`gwtd issue spec <n>`, file paths,
  memory headings).
- In `all_terms` mode, results may carry `matched_terms` / `missing_terms`,
  and semantic non-strict hits arrive in `suggestions`.
- Without `--json`, a human-readable `[scope] distance title — subtitle` list
  is printed instead.

## Interpreting results

- Lower distance values indicate higher relevance (0.0 = exact match)
- Uses semantic similarity, not just keyword matching
- Results are merged across scopes and ranked by distance
- The embedding model is `intfloat/multilingual-e5-base` (multilingual; handles Japanese)

## Auto-build fallback

When a target index does not exist, the search builds it inline (full mode)
and then returns results — the first call on a fresh checkout may take
noticeably longer. To force a rebuild explicitly:

```bash
"$GWT_BIN" index rebuild --scope all
"$GWT_BIN" index rebuild --scope issues   # or specs|memory|discussions|board|files|files-docs
```

## Empty corpus is a tooling failure, not "no results"

SPEC and Issue searches build their corpus from the GitHub Issue cache
(`~/.gwt/cache/issues/<repo-hash>/`). When that cache is empty or unpopulated,
the search fails with an `EMPTY_CORPUS` error instead of silently returning an
empty list — an empty list would read as "no existing SPEC/Issue owner" and
cause duplicate creation.

When you see `EMPTY_CORPUS`, **do not conclude that no owner exists.** Refresh
the issue cache (open the project in the gwt GUI to sync, or run
`gwtd issue spec pull --all`) and retry the search. Only a successful result
with an empty list from a *populated* cache means the repository genuinely has
no matching SPEC/Issue.

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

1. Resolve `GWT_BIN`, then run `"$GWT_BIN" search ... --json` with 2-3
   semantic queries derived from the request
2. A missing index is auto-built on the first call
3. Pick the canonical existing spec, issue, or memory if found
4. Only fall back to creating a new spec or issue when no suitable canonical match exists

## Fallback: direct runner invocation (older binaries only)

Only when `"$GWT_BIN" search` fails with `unknown command 'search'` (a gwtd
binary older than the search family, SPEC-1942 US-15), call the embedded
Python runner directly:

```bash
PYTHON=~/.gwt/runtime/chroma-venv/bin/python3   # Windows: ~/.gwt/runtime/chroma-venv/Scripts/python.exe
RUNNER=~/.gwt/runtime/chroma_index_runner.py

$PYTHON $RUNNER \
  --action search-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

Scope actions: `search-specs` / `search-issues` / `search-files` /
`search-files-docs` / `search-memory`. `search-specs`, `search-files`, and
`search-files-docs` additionally take `--worktree-hash "$GWT_WORKTREE_HASH"`.
`--match-mode all_terms` is accepted by every search action.

The hashes are an optimization, not a requirement: when `GWT_REPO_HASH` /
`GWT_WORKTREE_HASH` are unset or passed empty (e.g. when the launch
environment did not export them), the runner derives them from
`--project-root` automatically (Issue #2933) — no manual hash recomputation,
and no dependency on `sha256sum` (absent on stock macOS).

The fallback returns legacy per-scope shapes (`specResults` / `issueResults` /
`results` / `memoryResults`) instead of the unified `results` array, and
emits NDJSON auto-build progress on stderr. Pass `--no-auto-build` to disable
inline index builds; the runner then returns
`{"ok": false, "error_code": "INDEX_MISSING", ...}`.
