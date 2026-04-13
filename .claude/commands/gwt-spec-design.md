description: Compatibility alias for the visible SPEC design entrypoint
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

SPEC Design Command (Compatibility Alias)
========================================

Legacy command alias. Prefer `/gwt:gwt-design-spec` for visible SPEC design work.

Usage
-----

```text
/gwt:gwt-spec-design [args]
```

Steps
-----

1. Load `.claude/skills/gwt-design-spec/SKILL.md` and follow the visible workflow.
2. Reuse the owner SPEC when one already exists.
3. Produce a planning-ready SPEC for the next planning step.

Examples
--------

```text
/gwt:gwt-spec-design この機能を設計したい
```

```text
/gwt:gwt-spec-design --deepen SPEC-5
```

```text
/gwt:gwt-spec-design terminal multiplexing feature
```
