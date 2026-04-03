---
description: Brainstorm a rough request before SPEC registration and route it to the correct owner workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Brainstorm Command

Use this command when the user starts from a rough idea and needs interview-driven intake before any SPEC or Issue owner is chosen.

## Usage

```text
/gwt:gwt-spec-brainstorm [rough idea]
```

## Steps

1. Load `.claude/skills/gwt-spec-brainstorm/SKILL.md` and follow the workflow.
2. Search existing Issues and SPECs before proposing new work.
3. Interview the user one question at a time until the routing decision is clear.
4. Produce an `Intake Memo` and `Registration Decision` summary.
5. Continue automatically into `gwt-spec-ops`, `gwt-spec-register`, or `gwt-issue-register` based on that decision.

## Examples

```text
/gwt:gwt-spec-brainstorm Project Index を改善したい
```

```text
/gwt:gwt-spec-brainstorm SPEC にする前にこのアイデアを壁打ちしたい
```
