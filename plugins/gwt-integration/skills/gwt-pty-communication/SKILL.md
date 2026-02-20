---
name: gwt-pty-communication
description: PTY based communication tools for Project Mode orchestration (Lead/Coordinator/Developer).
---

# gwt PTY Communication

Use gwt terminal commands as the transport for agent-to-agent communication.

## Commands

- `send_keys_to_pane`: send text to a specific pane.
- `send_keys_broadcast`: send text to all running panes.
- `capture_scrollback_tail`: read pane output for status/progress.
- `list_terminals`: list active pane ids.
- `close_terminal`: stop a pane when escalation is needed.

## Notes

- Prefer targeted `send_keys_to_pane` for deterministic orchestration.
- Use `capture_scrollback_tail` before sending follow-up instructions.
