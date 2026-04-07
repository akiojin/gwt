---
name: gwt-project-index
description: Compatibility-oriented reference for the project file index. Prefer gwt-project-search for standalone semantic project search.
---

# Project Structure Index

gwt maintains a vector search index of project implementation files plus a separate docs collection using ChromaDB embeddings.

## File search command

Run in terminal to find files related to a feature or concept:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-files \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe` as the Python executable.

## File search output format

JSON object with ranked results:

```json
{"ok": true, "results": [
  {"path": "src/git/issue.rs", "description": "GitHub Issue commands", "distance": 0.12},
  {"path": "src/lib/components/IssuePanel.svelte", "description": "Issue list panel", "distance": 0.25}
]}
```

## When to use

- Task start: search for files related to the assigned feature
- Bug investigation: find files that might relate to the bug
- Feature addition: locate existing similar implementations
- Architecture understanding: discover how components are organized

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)

## Notes

- File index is auto-generated when the project is opened in gwt
- `search-files` targets the implementation-file collection; embedded skill assets, SPEC trees, local task logs, and snapshots are excluded from that collection
- Project docs are indexed separately and can be searched with `search-files-docs`
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- Canonical standalone file-search skill: `gwt-project-search`
- For Issue search, use `gwt-issue-search` instead
