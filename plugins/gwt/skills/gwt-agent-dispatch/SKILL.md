---
name: gwt-agent-dispatch
description: PTY-based instruction dispatch from Assistant to Agent panes. Send commands, capture output, and manage Agent lifecycle.
---

# gwt Agent Dispatch

Use gwt terminal commands to dispatch instructions from Assistant to Agent panes.

## Commands

- `send_keys_to_pane`: send instructions to a specific Agent pane.
- `send_keys_broadcast`: send instructions to all running Agent panes.
- `capture_scrollback_tail`: read Agent pane output for status/progress.
- `list_terminals`: list active Agent pane ids.
- `close_terminal`: stop an Agent pane when escalation is needed.

## Notes

- Prefer targeted `send_keys_to_pane` for deterministic dispatch.
- Use `capture_scrollback_tail` before sending follow-up instructions.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. PTY commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
- `GWT_PANE_ID`: pane ID of the current terminal session.
- `GWT_BRANCH`: branch name of the current session.
- `GWT_AGENT`: agent name of the current session.
