---
description: Semantic search over reusable project memory in `tasks/memory.md`
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Memory Search Command

Run a vector-embedded semantic search over reusable project memory recorded at
`tasks/memory.md`. Use before starting work that resembles a past failure,
when you want to reuse a previously verified prevention strategy, or when you
need prior design/workflow decisions.

## Usage

```text
/gwt:gwt-memory-search [query]
```

## Steps

1. Load `.claude/skills/gwt-memory-search/SKILL.md` and follow the workflow.
2. Execute the search query against the memory index.
3. Return ranked results with `memory_id`, `date`, `title`, `heading`,
   `chunk_idx`, and `distance`. Lower distance values are more relevant.

## Examples

```text
/gwt:gwt-memory-search "watcher debounce silent failure"
```

```text
/gwt:gwt-memory-search "spec section marker"
```

For a unified search that also includes SPECs, Issues, and project files, use
`/gwt:gwt-search "query"` (default merge) or `/gwt:gwt-search --memory "query"`
to keep only memory results. `/gwt:gwt-lessons-search` is a legacy alias.
