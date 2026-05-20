---
description: Legacy alias for semantic search over reusable project memory
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Lessons Search Command (Legacy Alias)

Run the legacy lessons search path against the canonical memory index. New
guidance should prefer `/gwt:gwt-memory-search`; this command remains for older
requests that use lessons terminology.

## Usage

```text
/gwt:gwt-lessons-search [query]
```

## Steps

1. Load `.claude/skills/gwt-lessons-search/SKILL.md` and follow the workflow.
2. Execute the search query against the memory index via the lessons alias.
3. Return ranked results with `date`, `title`, `heading`, `chunk_idx`, and
   `distance`. Lower distance values are more relevant.

## Examples

```text
/gwt:gwt-lessons-search "watcher debounce silent failure"
```

```text
/gwt:gwt-lessons-search "spec section marker"
```

For new usage, prefer `/gwt:gwt-memory-search "query"` or
`/gwt:gwt-search --memory "query"`.
