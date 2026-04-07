---
description: Compatibility alias for gwt-file-search
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Project Search Command

Use this compatibility command when older docs or habits still refer to `gwt-project-search`.
Prefer `/gwt:gwt-file-search` for new usage.

## Usage

```text
/gwt:gwt-project-search [query]
```

## Steps

1. Load `.claude/skills/gwt-file-search/SKILL.md` and follow the workflow.
2. If index status is unknown, check index health before searching.
3. Run semantic search and return top results with short rationale:
   - path
   - relevance summary
   - next file(s) to inspect
4. If index is missing or outdated, explain that and provide the shortest recovery action.
5. Prefer `/gwt:gwt-file-search` in new docs and prompts.

## Examples

```text
/gwt:gwt-project-search where branch naming is built
```

```text
/gwt:gwt-file-search project mode pty orchestration
```
