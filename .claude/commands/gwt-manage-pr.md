---
description: Create, inspect, update, or unblock a PR through the visible PR workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Manage PR Command
=================

Public task entrypoint for PR lifecycle work.

Usage
-----

```text
/gwt:gwt-manage-pr [action or context]
```

Steps
-----

1. Load `.claude/skills/gwt-manage-pr/SKILL.md` and follow the workflow.
2. Use the current branch and PR state to choose create, status, or unblock actions.
3. Keep PR work behind this visible entrypoint.

Examples
--------

```text
/gwt:gwt-manage-pr
```

```text
/gwt:gwt-manage-pr check status
```

```text
/gwt:gwt-manage-pr fix
```
