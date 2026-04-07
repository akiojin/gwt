---
description: Semantic search over project source files using the gwt-file-search skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT File Search Command

Use this command to run semantic search against the project file index.

## Usage

```text
/gwt:gwt-file-search [query]
```

## Steps

1. Load `.claude/skills/gwt-file-search/SKILL.md` and follow the workflow.
2. If index status is unknown, check index health before searching.
3. Run semantic search and return top results with short rationale:
   - path
   - relevance summary
   - next file(s) to inspect
4. If index is missing or outdated, explain that and provide the shortest recovery action.
5. For Issue search, use `/gwt:gwt-issue-search` instead.

## Examples

```text
/gwt:gwt-file-search where branch naming is built
```

```text
/gwt:gwt-file-search project mode pty orchestration
```
