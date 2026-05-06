---
description: Unified agent pane management for listing, reading, and stopping
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Agent Command

Unified agent pane management. Auto-detects mode from arguments: no args lists panes, pane ID reads output, and `stop`/`close` with pane ID stops a pane.

Use Board for agent-to-agent communication:

```bash
gwtd board post --kind request --target <session-id|branch|agent-id> --body '<message>'
gwtd board post --kind handoff --target <session-id|branch|agent-id> --body '<message>'
```

## Usage

```text
/gwt:gwt-agent [pane-id] [action]
```

## Steps

1. Load `.claude/skills/gwt-agent/SKILL.md` and follow the workflow.
2. Auto-detect the appropriate action based on provided arguments.
3. Execute the pane management operation, or route coordination through Board.

## Examples

```text
/gwt:gwt-agent
```

```text
/gwt:gwt-agent pane-id
```

```text
/gwt:gwt-agent stop pane-id
```
