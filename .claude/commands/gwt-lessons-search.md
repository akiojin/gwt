---
description: Semantic search over the project's `tasks/lessons.md` post-mortem entries
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Lessons Search Command

Run a vector-embedded semantic search over the post-mortem lessons recorded
at `tasks/lessons.md`. Use before starting work that resembles a past
failure, when you want to reuse a previously-verified prevention strategy,
or when you suspect the current bug has already been documented.

## Usage

```text
/gwt:gwt-lessons-search [query]
```

## Steps

1. Load `.claude/skills/gwt-lessons-search/SKILL.md` and follow the workflow.
2. Execute the search query against the lessons index.
3. Return ranked results with `date`, `title`, `heading`, `chunk_idx`, and
   `distance`. Lower distance values are more relevant.

## Examples

```text
/gwt:gwt-lessons-search "watcher debounce silent failure"
```

```text
/gwt:gwt-lessons-search "spec section マーカー"
```

For a unified search that also includes SPECs, Issues, and project files,
use `/gwt:gwt-search "query"` (default merge) or `/gwt:gwt-search --lessons
"query"` to keep only lesson results.
