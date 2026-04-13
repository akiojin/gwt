description: Compatibility alias for the visible PR lifecycle entrypoint
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

PR Command (Compatibility Alias)
===============================

Legacy command alias. Prefer `/gwt:gwt-manage-pr` for visible PR lifecycle work.

Usage
-----

```text
/gwt:gwt-pr [action or context]
```

Steps
-----

1. Load `.claude/skills/gwt-manage-pr/SKILL.md` and follow the visible workflow.
2. Ensure GitHub auth is healthy; `gwt pr current` should succeed before deeper PR actions.
3. Auto-detect the appropriate action based on current branch and PR state.
4. If the current PR is conflicting or behind, prefer the fix path over push-only.

Examples
--------

```text
/gwt:gwt-pr
```

```text
/gwt:gwt-pr fix
```

```text
/gwt:gwt-pr check status
```
