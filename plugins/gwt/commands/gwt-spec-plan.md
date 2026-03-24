---
description: Generate `plan.md` and supporting planning artifacts for an existing gwt-spec, including a constitution check.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Plan Command

Use this command after `spec.md` is clarification-ready.

## Usage

```text
/gwt:gwt-spec-plan [spec-issue-number|context]
```

## Steps

1. Load `skills/gwt-spec-plan/SKILL.md` and follow the workflow.
2. Read `.gwt/memory/constitution.md` and the target `spec.md` artifact.
3. Generate or update `plan.md`, `research.md`, `data-model.md`, `quickstart.md`, and `contract:*` artifacts.
4. Record constitution findings in `plan.md`.
5. Return control to `gwt-spec-ops` or continue into `gwt-spec-tasks`.
