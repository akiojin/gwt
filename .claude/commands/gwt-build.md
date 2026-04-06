---
description: Implement code using test-first TDD methodology
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Build Command

Implement code using test-first TDD methodology. Works in two modes: SPEC mode (driven by tasks.md with full progress tracking) and standalone mode (user-provided task, no SPEC artifacts needed).

## Usage

```text
/gwt:gwt-build [SPEC-ID or task description]
```

## Steps

1. Load `.claude/skills/gwt-build/SKILL.md` and follow the workflow.
2. In SPEC mode, read tasks.md and execute tasks in order with progress tracking.
3. In standalone mode, implement the user-provided task with TDD.

## Examples

```text
/gwt:gwt-build SPEC-5
```

```text
/gwt:gwt-build
```

```text
/gwt:gwt-build add clipboard support to the editor widget
```
