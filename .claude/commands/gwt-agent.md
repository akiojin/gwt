---
description: Unified agent pane management for listing, reading, sending, and stopping
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Agent Command

Unified agent pane management. Auto-detects mode from arguments: no args lists panes, pane ID reads output, pane ID with message sends input, 'stop'/'close' with pane ID stops a pane.

## Usage

```text
/gwt:gwt-agent [pane-id] [action] [message]
```

## Steps

1. Load `.claude/skills/gwt-agent/SKILL.md` and follow the workflow.
2. Auto-detect the appropriate action based on provided arguments.
3. Execute the pane management operation.

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
