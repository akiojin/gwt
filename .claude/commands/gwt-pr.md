---
description: Unified GitHub PR lifecycle manager for creation, status checks, and fixes
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# PR Command

Unified GitHub PR lifecycle manager. Auto-detects mode: creates PRs when none exist, pushes to open PRs, checks PR status, and fixes CI failures/merge conflicts/reviewer comments.

## Usage

```text
/gwt:gwt-pr [action or context]
```

## Steps

1. Load `.claude/skills/gwt-pr/SKILL.md` and follow the workflow.
2. Ensure GitHub auth is healthy; `gwt pr current` should succeed before deeper PR actions.
3. Auto-detect the appropriate action based on current branch and PR state.

## Examples

```text
/gwt:gwt-pr
```

```text
/gwt:gwt-pr fix
```

```text
/gwt:gwt-pr check status
```
