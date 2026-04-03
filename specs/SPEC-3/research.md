# Research: SPEC-3 - Agent Management

## Context
- Version cache startup is now wired so the wizard can render cached versions before refresh completes.
- The cache is a local JSON file in `~/.gwt/cache/agent-versions.json` with background refresh scheduling.
- Session conversion currently has a picker, confirm flow, and session metadata swap in the TUI.
- True PTY handoff and process-level replacement still need explicit reconciliation against the original acceptance language.
