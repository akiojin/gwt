---
description: Semantic search over GitHub gwt-spec Issues using the gwt-issue-search skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Issue Search Command

Use this command to run semantic search against the GitHub Issues index.

## Usage

```text
/gwt:gwt-issue-search [query]
```

## Steps

1. Load `skills/gwt-issue-search/SKILL.md` and follow the workflow.
2. If index status is unknown, check index health before searching.
3. Run semantic Issue search and return top results with short rationale:
   - issue number and title
   - relevance summary
   - next action
4. If index is missing/outdated, explain that and provide the shortest recovery action.

## Examples

```text
/gwt:gwt-issue-search project index spec
```

```text
/gwt:gwt-issue-search terminal pty orchestration spec
```

```text
/gwt:gwt-issue-search chroma recovery crash handling
```
