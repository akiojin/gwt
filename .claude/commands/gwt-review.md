---
description: Scan codebase and generate a prioritized architecture improvement report
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Architecture Review Command

Scan codebase structure, analyze domain boundaries (DDD), evaluate module depth (Ousterhout), assess agent-friendliness, and generate a prioritized improvement report.

## Usage

```text
/gwt:gwt-review [path]
```

## Steps

1. Load `.claude/skills/gwt-review/SKILL.md` and follow the workflow.
2. Analyze the target path or full codebase for architectural concerns.
3. Generate a prioritized report with actionable recommendations.

## Examples

```text
/gwt:gwt-review
```

```text
/gwt:gwt-review src/
```

```text
/gwt:gwt-review crates/gwt-core/
```
