---
description: Create or deepen a SPEC through the visible SPEC design workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Design SPEC Command
===================

Public task entrypoint for SPEC design.

Usage
-----

```text
/gwt:gwt-design-spec [args]
```

Steps
-----

1. Load `.claude/skills/gwt-design-spec/SKILL.md` and follow the workflow.
2. Reuse the owner SPEC when one already exists.
3. Leave the SPEC planning-ready for the next planning step.

Examples
--------

```text
/gwt:gwt-design-spec この機能を設計したい
```

```text
/gwt:gwt-design-spec --deepen SPEC-5
```
