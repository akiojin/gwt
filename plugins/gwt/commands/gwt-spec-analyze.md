---
description: Analyze a gwt-spec artifact set for completeness and consistency before implementation starts.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Analyze Command

Use this command as the final gate before implementation.

## Usage

```text
/gwt:gwt-spec-analyze [spec-issue-number|context]
```

## Steps

1. Load `skills/gwt-spec-analyze/SKILL.md` and follow the workflow.
2. Check `spec.md`, `plan.md`, `tasks.md`, and `memory/constitution.md`.
3. Report missing traceability, unresolved clarifications, or constitution gaps.
4. If clear, return control to `gwt-spec-ops`.
5. If blocked, point to the exact preceding skill that must run next.
