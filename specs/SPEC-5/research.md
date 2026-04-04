# Research: SPEC-5 - Local SPEC Management

## Scope Snapshot
- Canonical scope: Listing, detail, search, edit, and agent-launch behavior for local SPEC directories.
- Current status: `in-progress` / `Implementation`.
- Task progress: `50/50` checked in `tasks.md`.
- Notes: The live shell now exposes the Specs tab again, so the remaining work has shifted from reachability gaps to reviewer/completion evidence.

## Decisions
- Keep this SPEC focused on local artifact discovery and editing, not on generic management-tab routing.
- Keep search inside the Specs tab rather than adding a separate command-only flow.
- Prefer local ranking over ChromaDB so the search slice stays inside the TUI write set.

## Open Questions
- Completion-gate review still needs reviewer evidence on the current branch.
