---
name: gwt-agent-read
description: "Read the scrollback tail of an agent pane to check progress and status. Use when user says 'check pane output', 'read agent output', 'what is the agent doing?', or when monitoring agent progress."
---

# gwt Agent Read

Read the scrollback output of a specific agent pane.

## Commands

- `pane read <id> [--lines N]`: read the last N lines (default 50) of the specified pane's scrollback.

## Workflow

1. Run `pane list` (via `gwt-agent-discover`) to find the target pane ID.
2. Run `pane read <id>` to inspect the pane's recent output.
3. Analyze the output to determine agent progress or status.

## Notes

- Always discover panes first before reading.
- Use this skill before sending follow-up instructions via `gwt-agent-send`.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
