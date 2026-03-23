---
name: gwt-issue-search
description: Semantic search over GitHub gwt-spec Issues using vector embeddings. Use when searching for existing specs, finding related gwt-spec issues, checking for duplicate specs, or determining which spec owns a scope. Mandatory preflight before gwt-spec-register, gwt-spec-ops, gwt-issue-register, and gwt-issue-resolve.
---

# Issue Search

gwt maintains a vector search index of GitHub Issues using ChromaDB embeddings.

## Issues search first for spec integration

When the user asks any of the following, use GitHub Issues search **before** manual `gh issue list`,
title grep, or file search:

- "既存仕様を探して"
- "どの SPEC に統合するべきか"
- "関連 Issue / spec を探して"
- "Project Index の統合仕様を確認して"
- "bug / feature の過去設計を見たい"

For spec integration work, the first question is not "which file should I edit?" but
"which existing `gwt-spec` Issue is the canonical destination?".

Minimum workflow:

1. Update the Issues index with `index-issues`
2. Run `search-issues` with 2-3 semantic queries derived from the request
3. Pick the canonical existing spec if found
4. Only fall back to creating a new spec when no suitable canonical spec exists

Suggested query patterns:

- subsystem + purpose
  - `project index issue search spec`
- user-facing problem + architecture term
  - `chroma persisted db recovery project index`
- workflow / discoverability requirement
  - `LLM should use gwt-issue-search before spec creation`

## GitHub Issues search command

First, update the Issues index (fetches `gwt-spec` Issues via `gh` CLI):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-issues \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index"
```

Then search Issues semantically:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-issues \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index" \
  --query "your search query" \
  --n-results 10
```

## Issues search output format

```json
{"ok": true, "issueResults": [
  {"number": 42, "title": "Add vector search for Issues", "url": "https://github.com/...", "state": "open", "labels": ["gwt-spec"], "distance": 0.08}
]}
```

## When to use

- Spec integration: find the canonical `gwt-spec` Issue before creating or updating a spec
- Task start: search for Issues related to the assigned feature
- Bug investigation: find spec Issues that might relate to the bug
- Feature addition: locate relevant specs for similar implementations

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)

## Notes

- Issue index must be updated manually (via GUI "Update Index" button or `index-issues` action)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For file search, use `gwt-file-search` instead
