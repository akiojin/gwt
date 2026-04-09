---
description: Semantic search over local SPEC files using the gwt-spec-search skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Search Command

Use this command to run semantic search against the local SPEC index.

## Usage

```text
/gwt:gwt-spec-search [query]
```

## Steps

1. Load `skills/gwt-spec-search/SKILL.md` and follow the workflow.
2. Run semantic search and return top results with short rationale:
   - spec_id, title, status, phase
   - relevance summary
   - next spec(s) to inspect
3. For Issue search, use `/gwt:gwt-search --issues` instead.
4. For file search, use `/gwt:gwt-project-search` instead.

## Examples

```text
/gwt:gwt-spec-search workflow completion gate
```

```text
/gwt:gwt-spec-search GitHub PR management
```
