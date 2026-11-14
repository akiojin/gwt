---
description: Unified agent pane management for listing, reading, and stopping
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Agent Command

Unified agent pane management. Auto-detects mode from arguments: no args lists panes, pane ID reads output, and `stop`/`close` with pane ID stops a pane.

Use Board for agent-to-agent communication:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"board.post","params":{"kind":"request","targets":["<session-id|branch|agent-id>"],"body":"<message>"}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"board.post","params":{"kind":"handoff","targets":["<session-id|branch|agent-id>"],"body":"<message>"}}
JSON
```

## Usage

```text
/gwt:gwt-agent [pane-id] [action]
```

## Steps

1. Load `.claude/skills/gwt-agent/SKILL.md` and follow the workflow.
2. Auto-detect the appropriate action based on provided arguments.
3. Resolve `gwtd` as `GWT_BIN` from `GWT_BIN_PATH`, `command -v gwtd`, or `target/debug/gwtd`.
4. Execute pane management through JSON operations `pane.list`, `pane.read`, or `pane.close`, and route coordination through Board.

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
