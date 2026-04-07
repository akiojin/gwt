---
name: gwt-project-search
description: "Compatibility alias for gwt-file-search. Use when older docs or prompts still refer to gwt-project-search for semantic file search."
---

# Project Search Alias

`gwt-project-search` is a compatibility alias for `gwt-file-search`.
Prefer `gwt-file-search` in new docs, prompts, and slash-command usage.

## Canonical workflow

Use the same file-search workflow as `gwt-file-search`:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-files \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe` as the Python executable.

## Notes

- Canonical standalone skill name: `gwt-file-search`
- Canonical runner actions: `search-files` / `index-files`
- For SPEC search, use `gwt-spec-search`
- For Issue search, use `gwt-issue-search`
