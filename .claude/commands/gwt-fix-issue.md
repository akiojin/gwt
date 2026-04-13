---
description: Resolve an existing GitHub Issue through the visible issue-first workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Fix Issue Command
=================

Public task entrypoint for resolving an existing GitHub Issue.

Usage
-----

```text
/gwt:gwt-fix-issue <issue number or URL>
```

Steps
-----

1. Load `.claude/skills/gwt-fix-issue/SKILL.md` and follow the workflow.
2. Inspect the issue and decide direct-fix vs SPEC-needed.
3. Continue through implementation unless the work clearly needs a SPEC first.

Examples
--------

```text
/gwt:gwt-fix-issue #123
```

```text
/gwt:gwt-fix-issue https://github.com/akiojin/gwt/issues/123
```
