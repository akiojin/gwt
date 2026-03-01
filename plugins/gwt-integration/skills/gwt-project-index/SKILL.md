---
name: gwt-project-index
description: Semantic search over project files using vector embeddings.
---

# Project Structure Index

gwt maintains a vector search index of all project files using ChromaDB embeddings.

## Search command

Run in terminal to find files related to a feature or concept:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search \
  --db-path .gwt/index \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe` as the Python executable.

## Output format

JSON object with ranked results:

```json
{"ok": true, "results": [
  {"path": "src/git/issue.rs", "description": "GitHub Issue commands", "distance": 0.12},
  {"path": "src/lib/components/IssuePanel.svelte", "description": "Issue list panel", "distance": 0.25}
]}
```

## When to use

- Task start: search for files related to the assigned feature
- Bug investigation: find files that might contain the bug
- Feature addition: locate existing similar implementations
- Architecture understanding: discover how components are organized

## Notes

- Index is auto-generated when the project is opened in gwt
- Covers all files (source, docs, configs, specs)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
