---
name: gwt-memory-search
description: "Semantic search over reusable project memory at `tasks/memory.md` using vector embeddings. Use when looking for past post-mortem fixes, prior prevention notes, design decisions, workflow corrections, or before starting work that resembles an earlier failure. Legacy trigger phrases about lessons should use this skill; `gwt-lessons-search` remains an alias."
---

# Memory Search

gwt maintains a vector search index over reusable project memory entries kept
in `tasks/memory.md` using ChromaDB embeddings (model:
`intfloat/multilingual-e5-base`). Each H2 section is chunked and embedded. The
index is repo-scoped and stored at `~/.gwt/index/<repo-hash>/memory/`, shared
across worktrees.

`tasks/lessons.md` is a legacy fallback only. New entries belong in
`tasks/memory.md`.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Search before repeating known failures

Use memory search before writing new code or spec text when the work resembles
a past failure, review correction, or recurring design decision.

Minimum workflow:

1. Run `search-memory` with 2-3 semantic queries derived from the request.
2. Pick the most relevant past memory if one exists.
3. Read the matching section in `tasks/memory.md` before deciding the approach.
4. Reuse the existing prevention strategy when applicable.

## Search command

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-memory \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

If the memory index does not yet exist, the runner builds it inline (full mode)
from `<project_root>/tasks/memory.md` and emits NDJSON progress on stderr before
returning the search result.

To force a full re-index:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-memory \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full
```

`--worktree-hash` is accepted for symmetry with the other scopes but is ignored
because memory is repo-scoped. The legacy `search-lessons` and `index-lessons`
actions remain accepted and point to the same memory store.

## Output format

```json
{"ok": true, "memoryResults": [
  {"memory_id": "abc123def456", "lesson_id": "abc123def456", "date": "2026-05-20", "title": "example", "heading": "## 2026-05-20 - example", "chunk_idx": 0, "distance": 0.12}
]}
```

When a memory entry spans multiple chunks, only the best-scoring chunk per
`(date, title)` pair is surfaced. Use the `heading` field to locate the exact
section in `tasks/memory.md`.

## Write path

This skill does not write to `tasks/memory.md`. New entries must be added by
editing the file directly. Use this structure for new entries:

```markdown
## YYYY-MM-DD - short title

Type: lesson | decision | workflow | failure-pattern
Context: What happened or where the learning applies.
Learning: The reusable insight.
Future Action: What future agents should do differently.
```
