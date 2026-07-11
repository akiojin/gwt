---
description: Execute an Issue, gwt-spec Issue, or approved standalone task through the unified workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Execute Command
===============

Public task entrypoint for implementation work.

Usage
-----

```text
/gwt:gwt-execute [#issue | issue URL | task description]
```

Steps
-----

1. Load `.claude/skills/gwt-execute/SKILL.md` and follow the workflow.
2. Use `#N` for both plain Issues and gwt-spec tagged Issues.
3. Keep PR lifecycle work under `gwt-manage-pr`.

Examples
--------

```text
/gwt:gwt-execute #123
```

```text
/gwt:gwt-execute add clipboard support to the editor widget
```
