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

## Search command

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-lessons \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

The compatibility action reads the same repo-scoped memory index as
`search-memory`. It returns `lessonResults` for older callers and
`memoryResults` for newer callers.

## Write path

Use `gwtd memory add` for new reusable learning. `gwtd lessons add` remains a
legacy alias and writes the same canonical `tasks/memory.md` file:

```bash
"$GWT_BIN" memory add \
  --type lesson \
  --title "short title" \
  --context "What happened or where the learning applies." \
  --learning "The reusable insight." \
  --future-action "What future agents should do differently."
```

Direct file edits remain acceptable for unusual bulk edits or manual cleanup.
