# Research: SPEC-5 - Local SPEC Management

## Scope Snapshot
- Canonical scope: Listing, detail, search, edit, and agent-launch behavior for local SPEC directories.
- Current status: `in-progress` / `Implementation`.
- Task progress: `12/37` checked in `tasks.md`.
- Notes: The planned screen implementation is currently disconnected from the live management shell, so the docs must reflect that gap explicitly.

## Decisions
- Keep this SPEC focused on local artifact discovery and editing, not on generic management-tab routing.
- Document the current orphaned-state risk: code remains in tree, but the main shell no longer exposes a live Specs tab.
- Treat semantic search and persistent edit support as real remaining work, not as doc-only refresh items.

## Open Questions
- Decide whether local SPEC management returns as a first-class shell tab or moves behind another entry point.
- Confirm whether semantic search belongs in this screen flow or a separate command-first workflow.
