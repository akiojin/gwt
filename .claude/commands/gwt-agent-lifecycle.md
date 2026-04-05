---
description: Manage agent pane lifecycle using the gwt-agent-lifecycle skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Agent Lifecycle Command

Use this command to manage the lifecycle of agent panes.

## Usage

```text
/gwt:gwt-agent-lifecycle [pane-id]
```

## Steps

1. Load `.claude/skills/gwt-agent-lifecycle/SKILL.md` and follow the workflow.
2. Discover active panes first (`pane list`) to identify the target.
3. Confirm pane state (`pane read <id>`) before closing.
4. Run `pane close <id>` with an explicit reason.
