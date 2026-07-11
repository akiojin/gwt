---
description: Resolve an existing GitHub Issue through the visible issue-first workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Fix Issue Command
=================

Transition alias for the unified Execute command.

Usage
-----

```text
/gwt:gwt-fix-issue <issue number or URL>
```

Steps
-----

1. Prefer `/gwt:gwt-execute #N` for all Issue-backed execution.
2. Load `.claude/skills/gwt-execute/SKILL.md` and follow the workflow.
3. Keep PR lifecycle work under `gwt-manage-pr`.

Examples
--------

```text
/gwt:gwt-execute #123
```

```text
/gwt:gwt-execute https://github.com/akiojin/gwt/issues/123
```
