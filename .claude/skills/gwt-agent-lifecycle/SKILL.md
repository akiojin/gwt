---
name: gwt-agent-lifecycle
description: "This skill should be used when the user wants to stop or close an agent pane, says 'stop the agent', 'close pane', 'escalation needed', 'エージェントを止めて', 'ペインを閉じて', or when managing pane lifecycle due to stuck or unresponsive agents."
allowed-tools: Bash, Read
argument-hint: "[pane-id]"
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
