---
description: Implement an existing gwt-spec from `tasks.md`, keeping tests, issue progress, and PR flow moving until the scoped work is done.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Implement Command

Use this command after a SPEC is execution-ready.

## Usage

```text
/gwt:gwt-spec-implement [spec-issue-number|context]
```

## Steps

1. Load `skills/gwt-spec-implement/SKILL.md` and follow the workflow.
2. Read `spec.md`, `plan.md`, `tasks.md`, and the latest analysis result for the target `gwt-spec`.
3. Execute the next incomplete task slice in phase order, writing tests before code where behavior changes.
4. Update task/progress tracking and run the relevant verification commands.
5. Reconcile `spec.md`, `tasks.md`, `checklist:acceptance.md`, `checklist:tdd.md`, progress comments, and verification evidence before declaring the SPEC complete.
6. If reconciliation fails, return to `gwt-spec-ops` instead of leaving false completion markers behind.
7. Keep PR flow moving with `gwt-pr` and `gwt-pr-fix` until the scoped work is complete or a real decision blocker remains.
