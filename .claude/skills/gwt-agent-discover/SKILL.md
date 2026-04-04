---
name: gwt-agent-discover
description: "This skill should be used when the user wants to see active agent panes, says 'list panes', 'what agents are running?', 'show active agents', 'エージェント一覧', 'ペインを確認', or when discovering available panes before dispatch. It lists active agent panes with their IDs, agent types, branches, and statuses."
allowed-tools: Bash, Read
---

# gwt Agent Discover

List active agent panes scoped to the current project.

## Commands

- `pane list`: list active pane IDs with agent type, branch, and status.

## Workflow

1. Run `pane list` to enumerate active panes.
2. Use the output to select a target pane for `gwt-agent-read` or `gwt-agent-send`.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
