description: Compatibility alias for the visible discussion and design entrypoint
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

SPEC Design Command (Compatibility Alias)
========================================

Legacy command alias. Prefer `/gwt:gwt-discussion` for visible discussion and design work.

Usage
-----

```text
/gwt:gwt-spec-design [args]
```

Steps
-----

1. Load `.claude/skills/gwt-discussion/SKILL.md` and follow the visible workflow.
2. Reuse the owner SPEC when one already exists, or keep the work in discussion mode when it is not planning-ready yet.
3. Produce `Action Delta` / `Action Bundle` outputs consistent with `gwt-discussion`.

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
