# gwt Agent

Unified agent pane management: discover, read, and lifecycle operations.

Use the Board for agent-to-agent communication:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"board.post","params":{"kind":"request","targets":["<session-id|branch|agent-id>"],"body":"<message>"}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"board.post","params":{"kind":"handoff","targets":["<session-id|branch|agent-id>"],"body":"<message>"}}
JSON
```

Use `params.kind:"request"`, `"next"`, `"blocked"`, `"handoff"`, or
`"decision"` for coordination. `params.targets` highlights the entry as
`[for-you]` in the recipient's Board reminder injection. Omit `params.targets`
only for repo-wide Board updates.

Direct pane input is not part of the normal communication path. Prefer Board
posts so requests, decisions, blockers, and handoffs remain visible to every
agent and to the Workspace projection.

## gwtd Resolution

Resolve the `gwtd` executable once before running pane commands:

```bash
GWT_BIN="${GWT_BIN_PATH:-$(command -v gwtd || true)}"
if [ -z "$GWT_BIN" ] && [ -n "${GWT_PROJECT_ROOT:-}" ] && [ -x "$GWT_PROJECT_ROOT/target/debug/gwtd" ]; then
  GWT_BIN="$GWT_PROJECT_ROOT/target/debug/gwtd"
fi
if [ -z "$GWT_BIN" ] && [ -x "./target/debug/gwtd" ]; then
  GWT_BIN="./target/debug/gwtd"
fi
```

If `GWT_BIN` is empty, stop and report that `gwtd` could not be found.

## Mode Detection

Auto-detect the operation mode from arguments:

| Arguments | Mode | Operation |
|---|---|---|
| *(none)* or `list` | **Discover** | List active panes with IDs, agent types, branches, and statuses |
| `<pane-id>` | **Read** | Read the last 50 lines of the pane's scrollback |
| `<pane-id> {"lines":N}` | **Read** | Read the last N lines of the pane's scrollback |
| `stop <pane-id>` or `close <pane-id>` | **Lifecycle** | Stop and close the specified pane |
