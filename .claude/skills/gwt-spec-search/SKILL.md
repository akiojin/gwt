---
name: gwt-spec-search
description: "Semantic search over local SPEC files (specs/SPEC-{N}/) using vector embeddings. Use when searching for existing specs, finding related specs, checking for duplicate specs, or determining which spec owns a scope. Mandatory preflight before gwt-spec-register, gwt-spec-ops, gwt-issue-register, and gwt-issue-resolve. Use when user says 'search specs', 'find related specs', 'check for duplicate specs', or asks which spec owns a scope."
---

# SPEC Search

gwt maintains a vector search index of local SPEC files using ChromaDB embeddings.
The index is automatically updated when files in `specs/` change (via file system watcher).

## SPEC search first for spec integration

When the user asks any of the following, use SPEC search **before** manual file grep or directory listing:

- "既存仕様を探して"
- "どの SPEC に統合するべきか"
- "関連する SPEC を探して"
- "この機能の仕様は？"
- "重複する SPEC はないか確認して"

Minimum workflow:

1. Run `search-specs` with 2-3 semantic queries derived from the request
2. Pick the canonical existing spec if found
3. Only fall back to creating a new spec when no suitable canonical spec exists

## SPEC search command

Search SPECs semantically:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-specs \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index" \
  --query "your search query" \
  --n-results 10
```

To manually re-index (normally handled by file watcher):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-specs \
  --project-root "$GWT_PROJECT_ROOT" \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index"
```

## SPEC search output format

```json
{"ok": true, "specResults": [
  {"spec_id": "1579", "title": "gwt-spec system", "status": "open", "phase": "ready", "dir_name": "SPEC-1579", "distance": 0.08}
]}
```

## When to use

- Spec integration: find the canonical spec before creating or updating
- Task start: search for specs related to the assigned feature
- Duplicate check: verify no existing spec covers the same scope
- Architecture understanding: discover how features are specified

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)

## Notes

- SPEC index is automatically maintained by the file system watcher (changes to `specs/` trigger re-indexing)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For file search, use `gwt-file-search` instead
- For GitHub Issue search, use `gwt-issue-search` instead
