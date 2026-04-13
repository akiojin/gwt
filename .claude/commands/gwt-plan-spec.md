---
description: Generate or refresh SPEC planning artifacts through the visible planning workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Plan SPEC Command
=================

Public task entrypoint for SPEC planning.

Usage
-----

```text
/gwt:gwt-plan-spec [SPEC-ID]
```

Steps
-----

1. Load `.claude/skills/gwt-plan-spec/SKILL.md` and follow the workflow.
2. Generate or refresh the planning artifacts for the target SPEC.
3. Resolve planning gate issues before handing off to implementation.

Examples
--------

```text
/gwt:gwt-plan-spec SPEC-5
```

```text
/gwt:gwt-plan-spec
```
