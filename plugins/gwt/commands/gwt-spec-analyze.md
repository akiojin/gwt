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
2. Check `spec.md`, `plan.md`, `tasks.md`, and `.gwt/memory/constitution.md`.
3. Report missing traceability, unresolved clarifications, or constitution gaps, and classify them as `CLEAR`, `AUTO-FIXABLE`, or `NEEDS-DECISION`.
4. Treat `CLEAR` as execution readiness only, not as proof of final completion.
5. If the result is `CLEAR` or `AUTO-FIXABLE`, return control to `gwt-spec-ops`.
6. If the result is `NEEDS-DECISION`, point to the exact missing decision.
