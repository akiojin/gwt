---
description: Semantic search over project source files using the gwt-project-search skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Project Search Command

Use this command to run semantic search against the project structure index.

## Usage

```text
/gwt:gwt-project-search [query]
```

## Steps

1. Load `skills/gwt-project-search/SKILL.md` and follow the workflow.
2. If index status is unknown, check index health before searching.
3. Run semantic search and return top results with short rationale:
   - path
   - relevance summary
   - next file(s) to inspect
4. If index is missing/outdated, explain that and provide the shortest recovery action.
5. For Issue search, use `/gwt:gwt-issue-search` instead.

## Examples

```text
/gwt:gwt-project-search where branch naming is built
```

```text
/gwt:gwt-project-search project mode pty orchestration
```
