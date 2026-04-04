---
description: Read agent pane output using the gwt-agent-read skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Agent Read Command

Use this command to read the scrollback output of an agent pane.

## Usage

```text
/gwt:gwt-agent-read [pane-id]
```

## Steps

1. Load `.claude/skills/gwt-agent-read/SKILL.md` and follow the workflow.
2. Discover active panes first (`pane list`) if no pane ID is provided.
3. Run `pane read <id>` to inspect the pane's recent output.
