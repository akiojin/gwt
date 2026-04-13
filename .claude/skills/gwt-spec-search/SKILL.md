---
name: gwt-spec-search
description: "Semantic search over SPEC Issues (GitHub Issue cache at ~/.gwt/cache/issues/) using vector embeddings. Use when searching for existing specs, finding related specs, checking for duplicate specs, or determining which spec owns a scope. Mandatory preflight before gwt-design-spec and before issue-first workflows route into a SPEC owner. Use when user says 'search specs', 'find related specs', 'check for duplicate specs', or asks which spec owns a scope."
---

# SPEC Search

gwt maintains a vector search index of SPEC Issues using ChromaDB embeddings (model: `intfloat/multilingual-e5-base`). SPECs are stored as `gwt-spec` labeled GitHub Issues and cached locally at `~/.gwt/cache/issues/`. The index is stored at `~/.gwt/index/<repo-hash>/worktrees/<worktree-hash>/specs/` and is rebuilt from the cache. Use `gwt issue spec pull --all` to refresh the cache before searching.

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

## Environment

When the gwt TUI launches an agent pane, the following env vars are exported automatically:

- `GWT_PROJECT_ROOT` — absolute path of the active worktree
- `GWT_REPO_HASH` — SHA256[:16] of the normalized origin URL
- `GWT_WORKTREE_HASH` — SHA256[:16] of the canonicalized worktree absolute path

## SPEC search command

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-specs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

If the SPEC index does not yet exist, the runner builds it inline (full mode) and emits NDJSON progress on stderr before returning the search result.

To force a full re-index (normally handled by the watcher / auto-build):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-specs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full
```

## SPEC search output format

```json
{"ok": true, "specResults": [
  {"spec_id": "10", "title": "Project workspace", "status": "in-progress", "phase": "Implementation", "dir_name": "SPEC-10", "distance": 0.08}
]}
```

## When to use

- Spec integration: find the canonical spec before creating or updating
- Task start: search for specs related to the assigned feature
- Duplicate check: verify no existing spec covers the same scope
- Architecture understanding: discover how features are specified

## Notes

- SPEC index is maintained by the TUI watcher; non-TUI sessions get an mtime+size diff per call
- The runner auto-builds the index when missing (use `--no-auto-build` to suppress)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For file search, use `gwt-project-search` instead
- For GitHub Issue search, use `gwt-issue-search` instead
