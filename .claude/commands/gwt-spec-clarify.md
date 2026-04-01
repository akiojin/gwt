---
description: Resolve `[NEEDS CLARIFICATION]` markers and tighten `spec.md` for an existing gwt-spec before planning.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Clarify Command

Use this command after a SPEC container exists and before planning artifacts are generated.

## Usage

```text
/gwt:gwt-spec-clarify [spec-issue-number|context]
```

## Steps

1. Load `.claude/skills/gwt-spec-clarify/SKILL.md` and follow the workflow.
2. Read the `spec.md` artifact comment for the target `gwt-spec`.
3. Resolve `[NEEDS CLARIFICATION]` markers and weak acceptance scenarios, filling obvious gaps before asking the user.
4. If a real product or scope decision remains, stop with a clarification report.
5. Otherwise return control to `gwt-spec-ops` or proceed to `gwt-spec-plan`.
