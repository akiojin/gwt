---
name: gwt-issue-search
description: "Semantic search over all GitHub Issues using vector embeddings. Use when searching for existing issues, finding related issues, checking for duplicate issues, or determining which issue owns a scope. Mandatory preflight before gwt-spec-register, gwt-spec-ops, gwt-issue-register, and gwt-issue-resolve. Use when user says 'search issues', 'find related issues', 'check for duplicates', or asks which issue owns a scope."
---

# Issue Search

gwt maintains a vector search index of GitHub Issues using ChromaDB embeddings (model: `intfloat/multilingual-e5-base`). The index is stored at `~/.gwt/index/<repo-hash>/issues/` and is Worktree-independent. The gwt TUI refreshes it asynchronously at startup with a 15-minute TTL; non-TUI sessions get an auto-build on the first search.

## Issues search first

When the user asks any of the following, use GitHub Issues search before manual `gwt issue view`,
title grep, or file search:

- "既存 Issue を探して"
- "関連 Issue を探して"
- "Project Index の統合仕様を確認して"
- "bug / feature の過去設計を見たい"

Minimum workflow:

1. Run `search-issues` with 2-3 semantic queries derived from the request (the runner auto-builds the index if missing)
2. Pick the canonical existing issue if found
3. Only fall back to creating a new issue when no suitable canonical issue exists

Suggested query patterns:

- subsystem + purpose
  - `project index issue search spec`
- user-facing problem + architecture term
  - `chroma persisted db recovery project index`
- workflow / discoverability requirement
  - `LLM should use gwt-issue-search before spec creation`

## Environment

When the gwt TUI launches an agent pane, the following env vars are exported automatically:

- `GWT_PROJECT_ROOT` — absolute path of the active worktree
- `GWT_REPO_HASH` — SHA256[:16] of the normalized origin URL

## GitHub Issues search command

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

If the Issue index does not yet exist, the runner builds it inline (full mode) by refreshing
issue data and embedding the results, then performs the search.

To force a refresh ignoring TTL:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT"
```

Add `--respect-ttl` to skip when the previous refresh is younger than 15 minutes (used by the TUI startup background task).

## Issues search output format

```json
{"ok": true, "issueResults": [
  {"number": 42, "title": "Add vector search for Issues", "url": "https://github.com/...", "state": "open", "labels": ["enhancement"], "distance": 0.08}
]}
```

## When to use

- Issue lookup: find existing GitHub Issues before creating new ones
- Task start: search for Issues related to the assigned feature
- Bug investigation: find Issues that might relate to the bug
- Feature addition: locate relevant Issues for similar implementations

## Notes

- The TUI refreshes the Issue index automatically at startup (TTL 15 min). Non-TUI sessions trigger an inline build on the first search.
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For SPEC search, use `gwt-spec-search` instead (SPECs are now local files)
- For file search, use `gwt-project-search` instead
