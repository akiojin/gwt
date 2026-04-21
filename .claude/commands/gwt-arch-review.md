---
description: Scan codebase and generate a prioritized architecture improvement report
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Architecture Review Command

Scan codebase structure, analyze domain boundaries (DDD), evaluate module depth (Ousterhout), assess agent-friendliness, and generate a prioritized improvement report.

## Usage

```text
/gwt:gwt-arch-review --scope repo
/gwt:gwt-arch-review --scope changed --base <ref>
```

If omitted, prompt for the scope first. When `changed` is selected interactively, prompt for the base ref next.

## Steps

1. Load `.claude/skills/gwt-arch-review/SKILL.md` and follow the workflow.
2. Analyze either the full repository or the files changed since the selected base ref.
3. Generate a prioritized report with actionable recommendations.

## Examples

```text
/gwt:gwt-arch-review
```

```text
/gwt:gwt-arch-review --scope repo
```

```text
/gwt:gwt-arch-review --scope changed --base origin/develop
```
