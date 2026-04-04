---
name: gwt-agent-lifecycle
description: "Stop an agent pane when escalation is needed or the agent is stuck. Use when user says 'stop the agent', 'close pane', 'escalation needed', or when managing pane lifecycle."
---

# gwt Agent Lifecycle

Manage the lifecycle of agent panes.

## Commands

- `pane close <id>`: stop the specified pane.

## Workflow

1. Run `pane list` (via `gwt-agent-discover`) to identify the target pane.
2. Run `pane read <id>` (via `gwt-agent-read`) to confirm the pane is stuck or needs escalation.
3. Run `pane close <id>` to stop the pane with an explicit reason.

## Notes

- Only close panes when escalation is needed or the agent is unresponsive.
- Always confirm the pane state before closing.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
