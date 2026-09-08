---
description: Implement an approved SPEC or approved standalone task through the visible build workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Build SPEC Command
==================

Transition alias for the unified Execute command.

Usage
-----

```text
/gwt:gwt-build-spec [SPEC-ID or task description]
```

Steps
-----

1. Prefer `/gwt:gwt-execute #N` for all Issue-backed execution.
2. Load `.claude/skills/gwt-execute/SKILL.md` and follow the workflow.
3. Keep PR lifecycle work under `gwt-manage-pr`.

Examples
--------

```text
/gwt:gwt-execute #5
```

```text
/gwt:gwt-build-spec add clipboard support to the editor widget
```
