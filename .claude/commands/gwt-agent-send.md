---
description: Send input to agent panes using the gwt-agent-send skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Agent Send Command

Use this command to send key input to agent panes.

## Usage

```text
/gwt:gwt-agent-send [context]
```

## Steps

1. Load `.claude/skills/gwt-agent-send/SKILL.md` and follow the workflow.
2. Discover active panes first (`pane list`) before sending.
3. Check pane state (`pane read <id>`) before sending follow-up instructions.
4. Prefer targeted `pane send <id> <input>` over broadcast.
