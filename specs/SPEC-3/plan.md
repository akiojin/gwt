# Agent Management -- Implementation Plan

## Summary

Implement the version cache feature and complete the session conversion UI. Agent detection, launch wizard, Quick Start, and custom agent CRUD are already implemented and tested.

## Technical Context

- **Agent trait**: `crates/gwt-core/src/agent/` -- `AgentTrait::detect()`, agent registry
- **Launch wizard**: `crates/gwt-tui/src/screens/` -- wizard step components
- **Launch builder**: `crates/gwt-core/src/agent/launch.rs` -- `AgentLaunchBuilder`
- **Custom agents**: `crates/gwt-tui/src/screens/settings.rs` -- CRUD UI
- **Settings persistence**: `~/.gwt/config.toml` -- custom agent configuration
- **Quick Start history**: `crates/gwt-core/src/` -- per-branch launch history

## Constitution Check

- Spec before implementation: yes, this SPEC documents all agent management requirements.
- Test-first: version cache and session conversion tests must be RED before implementation.
- No workaround-first: version cache uses proper async fetch with TTL, not polling.
- Minimal complexity: cache is a simple JSON file with TTL check; no database needed.

## Complexity Tracking

- Added complexity: npm registry HTTP client, cache file management, async startup task
- Mitigation: single async task at startup, simple JSON schema, atomic file writes

## Phased Implementation

### Phase 1: Version Cache Implementation

1. Define cache schema: `{ agent_name: { versions: [...], fetched_at: ISO8601 } }`.
2. Implement npm registry client to fetch latest 10 versions for a given package name.
3. Implement cache read/write with atomic file operations and TTL check.
4. Spawn async cache refresh task on gwt startup (non-blocking).
5. Wire cached versions into the wizard's model selection step.
6. Add tests: cache read/write, TTL expiry, network failure fallback, corrupted file handling.

### Phase 2: Session Conversion UI

1. Add session conversion action to session context menu or keybinding.
2. Display available agent list (filtered to detected agents).
3. On confirmation, update the active session metadata to the selected agent while preserving repository context.
4. Handle conversion failure: keep the original session intact and display an error notification.
5. Add tests: conversion success path, conversion failure path, working directory preservation.

## Dependencies

- `reqwest` or `ureq` crate for HTTP client (npm registry fetch).
- `tokio` runtime (already in use) for async cache refresh.
- `serde_json` for cache file serialization (already a dependency).
