## Commands

### Discover

Run:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pane.list","params":{}}
JSON
```

Lists active pane IDs with agent type, branch, and status.

### Read

Run:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pane.read","params":{"id":"<id>","lines":50}}
JSON
```

Reads the last N lines (default 50) of the specified pane's scrollback.

### Coordinate

- `board.post` with `params.kind:"request"` and `params.targets:["<id>"]`:
  ask a specific agent to act or respond.
- `board.post` with `params.kind:"handoff"` and `params.targets:["<id>"]`:
  hand off context or next ownership.
- `board.post` with `params.kind:"blocked"`: expose a blocker and ask for
  unblock help.

### Lifecycle

Run:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pane.close","params":{"id":"<id>"}}
JSON
```

Stops the specified pane.
