---
name: gwt-project-search
description: "Semantic search over project source files using vector embeddings. Use when the user asks to search project files, find related implementation files, or locate source files for a feature, bug, or concept."
---

# Project Search

gwt maintains a vector search index of project source files using ChromaDB embeddings.
The index is automatically updated when source files change (via file system watcher).

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

- Task start: search for project files related to the assigned feature
- Bug investigation: find implementation files that might relate to the bug
- Feature addition: locate existing similar implementations in the project
- Architecture understanding: discover how project components are organized

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)

## Notes

- Project file index is automatically maintained by the file system watcher (changes trigger re-indexing)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- Canonical standalone skill name: `gwt-project-search`
- Internal runner actions remain `search-files` / `index-files`
- For SPEC search, use `gwt-spec-search` instead
- For Issue search, use `gwt-issue-search` instead
