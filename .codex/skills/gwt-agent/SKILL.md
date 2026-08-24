---
name: gwt-agent
description: "Use proactively when monitoring or controlling running agent panes or the Issue Monitor queue. Auto-detects pane mode from arguments. For agent-to-agent communication, use the shared Board."
allowed-tools: Bash, Read
argument-hint: "[list | <pane-id> [--lines N] | stop <pane-id>]"
---

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

## Issue Monitor Queue Operations

Use these JSON operations for project-scoped queue inspection and control. The
optional `project_root` parameter defaults to the caller's current worktree.

Read the ordered runtime queue and active launches:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.status","params":{}}
JSON
```

Move an Issue to the head (the default) or to a zero-based numeric index:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.priority.move","params":{"number":42,"position":"head"}}
JSON
```

Replace the complete priority order (an empty array clears it):

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.priority.set","params":{"issue_numbers":[42,17]}}
JSON
```

Safely stop processing or lower/raise the positive concurrency limit:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.config.set","params":{"enabled":false,"autonomous_mode":false,"max_active":2}}
JSON
```

Ask the Issue Monitor to take an Issue next — move it to the priority head and
trigger an immediate scan. The launch itself still goes through the Monitor's
own claim/slot path, so this can never produce a duplicate agent:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.launch_now","params":{"number":42}}
JSON
```

The response reports `scan_delivery`: `immediate` when a daemon accepted the
scan request, `next-scheduled-scan` when none was reachable (the new order is
already durable either way).

`enabled=true` and `autonomous_mode=true` are intentionally rejected for agent
sessions. Enabling either capability requires an explicit GUI action — the one
exception is the project's resident PM agent (SPEC-3431), which may raise them
from the CLI; run `pm.status` to see whether the current session holds that
privilege (`caller_is_registered_pm`). Configuration changes are
committed atomically to the project preferences source of truth; OFF operations
also revoke outstanding effect authority. Priority changes become visible to
the GUI and daemon on their next scan/rebase. Configuration changes use an
atomic daemon control when it is available; the fence-aware local fallback is
observed on the next scan.

### Checking which project store the operation reached

`ok: true` proves the operation ran; it does not prove *where*. A `project_root`
whose repository identity cannot be resolved still gets a working project store
— just an isolated one, keyed by path, that no running gwt opened under a
different path will ever read. A write there succeeds and reads straight back
while changing nothing the GUI or daemon sees.

Every `issue.monitor.*` response therefore carries a `project_store` block on
the envelope, next to `ok`:

```json
{
  "ok": true,
  "operation": "issue.monitor.status",
  "project_store": {
    "project_root": "/Users/you/Workbench/gwt",
    "hash": "99a8660247f5bc49",
    "source": "nested_bare_repository",
    "identity_resolved": true,
    "store_path": "/Users/you/.gwt/projects/99a8660247f5bc49"
  }
}
```

- `identity_resolved: true` — the store is keyed by the repository's origin, so
  every worktree and layout root of that repository shares it. `source` is
  `origin` or `nested_bare_repository`.
- `identity_resolved: false` — the store is isolated to this path. `source` is
  `path_fallback` (no repository identity was resolvable) or
  `ambiguous_nested_bare_repositories` (several direct-child bare repositories
  name different origins; the rejected `candidates` are listed).

Check `identity_resolved` and `hash` after any write that matters. A path
fallback usually means the `project_root` was a directory that only *contains*
the repository — pass the repository or a worktree inside it instead. Do not
infer the landing from file mtimes under `~/.gwt/projects/`.

## Parked Question Handoffs

An unattended autonomous execution never waits on a confirmation question. A
question tool call is converted into a structured handoff before it can wait:
the owner Issue is parked as `NeedsHuman` and its active slot is released for
the next ready Issue.

List the questions parked executions are waiting on:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.questions","params":{}}
JSON
```

Each entry carries the owner Issue, the asking session, the question text, the
offered options, the rationale, and a machine-readable `reason_code`.

Answer one parked question by its `handoff_id`:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.monitor.question.answer","params":{"handoff_id":"<id>","answer":"<decision>"}}
JSON
```

An unknown or already-answered `handoff_id` is rejected rather than silently
accepted. On the next scan the driver un-parks the Issue and resumes the exact
stored session with the answer as its first prompt — no duplicate launch.

## Workflows

### Discover Mode

1. Run JSON operation `pane.list` to enumerate active panes.
2. Present the list with pane IDs, agent types, branches, and statuses.

### Read Mode

1. Run JSON operation `pane.list` first if the pane ID is not already known.
2. Run JSON operation `pane.read` to inspect the pane's recent output.
3. Analyze the output to determine agent progress or status.

### Coordination Mode

1. Use JSON operation `pane.list` or recent Board context to identify the target session,
   branch, or agent ID.
2. Post to Board with JSON operation `board.post`.
3. Use `params.targets` for a specific recipient; omit it for repo-wide coordination.
4. Use `params.parent` when replying to an existing Board thread.

### Lifecycle Mode

1. Run JSON operation `pane.list` to identify the target pane if not already known.
2. Run JSON operation `pane.read` to confirm the pane is stuck or needs escalation.
3. Run JSON operation `pane.close` to stop the pane with an explicit reason.

## Notes

- Always discover panes before reading or closing.
- Read pane output before posting follow-up requests that depend on pane state.
- Prefer targeted Board posts over untargeted posts for deterministic handoff.
- Only close panes when escalation is needed or the agent is unresponsive.
- Always confirm pane state before closing.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
- `GWT_PANE_ID`: pane ID of the current pane.
- `GWT_BIN_PATH`: absolute path to the current `gwtd` binary injected by gwt launches when available.
