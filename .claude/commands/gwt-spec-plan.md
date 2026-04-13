description: Compatibility alias for the visible SPEC planning entrypoint
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

SPEC Planning Command (Compatibility Alias)
==========================================

Legacy command alias. Prefer `/gwt:gwt-plan-spec` for visible SPEC planning work.

Usage
-----

```text
/gwt:gwt-spec-plan [SPEC-ID]
```

Steps
-----

1. Load `.claude/skills/gwt-plan-spec/SKILL.md` and follow the visible workflow.
2. Read the target SPEC's spec.md and generate planning artifacts.
3. Resolve planning gate findings before implementation.

Examples
--------

```text
/gwt:gwt-spec-plan SPEC-5
```

```text
/gwt:gwt-spec-plan
```

```text
/gwt:gwt-spec-plan --lightweight
```
