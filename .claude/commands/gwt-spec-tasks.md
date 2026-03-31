---
description: Generate `tasks.md` for an existing gwt-spec from `spec.md` and `plan.md`, grouped by phase and user story.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Tasks Command

Use this command after planning artifacts exist.

## Usage

```text
/gwt:gwt-spec-tasks [spec-issue-number|context]
```

## Steps

1. Load `skills/gwt-spec-tasks/SKILL.md` and follow the workflow.
2. Read `spec.md`, `plan.md`, and supporting artifacts.
3. Generate `tasks.md` with phases, user story mapping, `[P]` markers, and exact paths.
4. Ensure verification tasks exist for each acceptance scenario.
5. Ensure the task plan leaves enough verification evidence for the later post-implementation completion gate.
6. Return control to `gwt-spec-ops` or continue into `gwt-spec-analyze`.
