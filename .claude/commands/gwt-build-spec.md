---
description: Implement an approved SPEC or approved standalone task through the visible build workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Build SPEC Command
==================

Public task entrypoint for implementation work.

Usage
-----

```text
/gwt:gwt-build-spec [SPEC-ID or task description]
```

Steps
-----

1. Load `.claude/skills/gwt-build-spec/SKILL.md` and follow the workflow.
2. Prefer SPEC mode when a SPEC exists.
3. Keep PR lifecycle work under `gwt-manage-pr`.

Examples
--------

```text
/gwt:gwt-build-spec SPEC-5
```

```text
/gwt:gwt-build-spec add clipboard support to the editor widget
```
