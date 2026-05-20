---
name: gwt-lessons-search
description: "Legacy alias for gwt-memory-search. Use for old requests that say lessons, past failures, or 過去 lesson を引いて; search the canonical `tasks/memory.md` memory index via the `search-lessons` compatibility action."
---

# Lessons Search (Legacy Alias)

`gwt-lessons-search` remains for compatibility with older commands and user
phrases. The canonical store is `tasks/memory.md`, indexed under
`~/.gwt/index/<repo-hash>/memory/`. Legacy `tasks/lessons.md` is used only as a
fallback when `memory.md` is absent.

Prefer `gwt-memory-search` and `search-memory` in new guidance.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Legacy lessons search workflow

When the user asks any of the following, use this compatibility path or switch
to `gwt-memory-search`:

- "過去 lesson を引いて"
- "同じ失敗があるか確認して"
- "before fixing X, check whether we have learned this before"
- "Has this regression been recorded?"

Minimum workflow:

1. Run `search-lessons` with 2-3 semantic queries derived from the request.
2. Pick the most relevant past memory if one exists.
3. Read the matching section in `tasks/memory.md` before deciding the approach.
4. Reuse the existing prevention strategy when applicable.

## Search command

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-lessons \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

If the memory index does not yet exist, the runner builds it inline (full mode)
from `<project_root>/tasks/memory.md` and emits NDJSON progress on stderr before
returning the search result. If `memory.md` is absent, legacy
`tasks/lessons.md` is used as a fallback source.

To force a full re-index (normally handled by the project watcher or the search
auto-build fallback):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-lessons \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full
```

`--worktree-hash` is accepted for symmetry with the other scopes but is ignored;
memory is repo-scoped and serves every worktree from a single index.

## Output format

```json
{"ok": true, "lessonResults": [
  {"date": "2026-05-20", "title": "example", "heading": "## 2026-05-20 - example", "chunk_idx": 0, "distance": 0.12}
]}
```

The compatibility action also returns `memoryResults` for newer callers. When a
memory entry spans multiple chunks, only the best-scoring chunk per
`(date, title)` pair is surfaced. Use the `heading` field to locate the exact
section in `tasks/memory.md`.

## Write path

This skill does **not** write to `tasks/memory.md`. New reusable learning must
be added by editing the file directly with `Type`, `Context`, `Learning`, and
`Future Action` fields. The watcher and the auto-build fallback pick up the
change automatically.

## Notes

- The runner auto-builds the memory index when missing (use `--no-auto-build`
  to suppress).
- Uses semantic similarity, not just keyword matching. Lower distance values
  indicate higher relevance.
- For SPEC search, use `gwt-spec-search`. For GitHub Issue search, use
  `gwt-issue-search`. For implementation file search, use `gwt-project-search`.
  For a unified result across all scopes, use `gwt-search` and add `--memory`
  (or omit filters to merge every scope).
