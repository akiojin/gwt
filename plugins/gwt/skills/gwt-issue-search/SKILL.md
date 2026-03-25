---
name: gwt-issue-search
description: Semantic search over all GitHub Issues using vector embeddings. Use when searching for existing issues, finding related issues, checking for duplicate issues, or determining which issue owns a scope. Mandatory preflight before gwt-spec-register, gwt-spec-ops, gwt-issue-register, and gwt-issue-resolve.
---

# Issue Search

gwt maintains a vector search index of GitHub Issues using ChromaDB embeddings.

## Issues search first

When the user asks any of the following, use GitHub Issues search **before** manual `gh issue list`,
title grep, or file search:

- "既存 Issue を探して"
- "関連 Issue を探して"
- "Project Index の統合仕様を確認して"
- "bug / feature の過去設計を見たい"

Minimum workflow:

1. Update the Issues index with `index-issues`
2. Run `search-issues` with 2-3 semantic queries derived from the request
3. Pick the canonical existing issue if found
4. Only fall back to creating a new issue when no suitable canonical issue exists

Suggested query patterns:

- subsystem + purpose
  - `project index issue search spec`
- user-facing problem + architecture term
  - `chroma persisted db recovery project index`
- workflow / discoverability requirement
  - `LLM should use gwt-issue-search before spec creation`

## GitHub Issues search command

First, update the Issues index (fetches all GitHub Issues via `gh` CLI):

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
  {"number": 42, "title": "Add vector search for Issues", "url": "https://github.com/...", "state": "open", "labels": ["enhancement"], "distance": 0.08}
]}
```

## When to use

- Issue lookup: find existing GitHub Issues before creating new ones
- Task start: search for Issues related to the assigned feature
- Bug investigation: find Issues that might relate to the bug
- Feature addition: locate relevant Issues for similar implementations

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)

## Notes

- Issue index must be updated manually (via GUI "Update Index" button or `index-issues` action)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For SPEC search, use `gwt-spec-search` instead (SPECs are now local files)
- For file search, use `gwt-project-search` instead
