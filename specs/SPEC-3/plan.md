# Agent Management -- Implementation Plan

## Summary

Implement the version cache feature and complete the session conversion UI.
Agent detection, launch wizard, Quick Start, and custom agent CRUD are
already implemented and tested. The remaining wizard work separates model
selection from version selection and materializes launch state into a
persisted agent session before activation.

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
5. Wire cached versions into a dedicated VersionSelect step rather than mixing
   them into model selection.
6. Add tests: cache read/write, TTL expiry, network failure fallback,
   corrupted file handling, installed-version de-duplication, and wizard
   option refresh.
7. Resolve launch runner choice from the selected version:
   `installed`/empty -> direct binary, `latest`/semver -> `bunx` or `npx`.

### Phase 2: Session Conversion UI

1. Add session conversion action to session context menu or keybinding.
2. Display available agent list (filtered to detected agents).
3. On confirmation, update the active session metadata to the selected agent while preserving repository context.
4. Handle conversion failure: keep the original session intact and display an error notification.
5. Add tests: conversion success path, conversion failure path, working directory preservation.

### Phase 3: Wizard Launch Materialization

1. Keep explicit model selection separate from default UI labels so launch
   flags only include real model identifiers.
2. Build a pending launch config from the wizard without holding a mutable
   borrow across app-level side effects.
3. Materialize the pending launch into a persisted `~/.gwt/sessions/*.toml`
   entry and activate the new agent tab.
4. Add focused tests for launch-config normalization and session persistence.

### Phase 4: Wizard UX Restoration

1. Restore the branch-first wizard flow so existing-branch launches begin at
   branch action and spec-prefilled launches begin at branch type selection.
2. Reorder new-branch setup to run Branch Type -> Issue -> AI naming ->
   Branch Name before agent selection while keeping the current Confirm step.
3. Restore the old branch type and execution mode labels in the current
   ratatui wizard without regressing version selection or spec-context AI
   prompts.
4. Add focused tests for branch-first transitions, spec-prefill startup, and
   the updated option labels.

## Dependencies

- `reqwest` or `ureq` crate for HTTP client (npm registry fetch).
- `tokio` runtime (already in use) for async cache refresh.
- `serde_json` for cache file serialization (already a dependency).
