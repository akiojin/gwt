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

### Phase 5: Old-TUI Wizard Step Machine Restoration

1. Replace the current shortcut-oriented step enum with the old-TUI-aligned
   step machine: `QuickStart`, `BranchAction`, `AgentSelect`, `ModelSelect`,
   `ReasoningLevel`, `VersionSelect`, `ExecutionMode`,
   `ConvertAgentSelect`, `ConvertSessionSelect`, `SkipPermissions`,
   `BranchTypeSelect`, `IssueSelect`, `AIBranchSuggest`, and
   `BranchNameInput`.
2. Rewrite `next_step()` and `prev_step()` to follow the old-TUI transition
   table while preserving the current backend hooks for version cache, AI
   branch suggestions, and session conversion.
3. Remove the separate `Confirm` step so that the final selection step
   completes directly.
4. Add focused RED/GREEN coverage for the new step transitions before
   touching popup rendering polish.

### Phase 6: Old-TUI Wizard Option Formatting

1. Restore old-TUI row formatting for `ModelSelect`, `ReasoningLevel`,
   `ExecutionMode`, and `SkipPermissions` so rows show aligned labels plus
   descriptions instead of plain labels only.
2. Restore old-TUI `VersionSelect` rendering with `label - description`
   formatting and `^ more above ^` / `v more below v` indicators when the
   list overflows.
3. Keep launch/config backend semantics unchanged while adding focused render
   tests before updating any snapshots.

### Phase 7: Quick Start History Restoration

1. Reconstruct per-branch Quick Start history from persisted agent sessions
   in `~/.gwt/sessions/`, grouping to the newest entry per agent for the
   current repository and branch.
2. Restore old-TUI `QuickStart` rendering with a branch summary row, colored
   agent headers, paired `Resume` / `Start new` actions, and a trailing
   `Choose different settings...` option.
3. Restore Quick Start selection semantics so `Resume` reuses the persisted
   resume session ID when available, otherwise falls back to `Continue`, and
   `Start new` keeps the previous configuration while resetting session
   continuity.
4. Cover the slice with RED/GREEN tests for history loading, Quick Start
   rendering, and launch-config restoration.

### Phase 8: Old-TUI AgentSelect and Popup Chrome

1. Restore old-TUI popup chrome by moving the current step title into the
   border, adding a right-aligned `[ESC]` hint, and keeping the content area
   centered without extra inner chrome.
2. Restore `AgentSelect` rendering so existing-branch launches show
   `Branch: ...` above the list and the agent rows render as name-only
   entries with the old-TUI selected-row highlight.
3. Add focused RED/GREEN coverage for popup chrome text and AgentSelect
   rendering before changing any other wizard steps.

### Phase 12: Old-TUI Inline Input Prompts

1. Restore old-TUI inline prompt rendering for `BranchNameInput` and
   `IssueSelect` so the popup chrome remains the only boxed title surface.
2. Keep the typed value in yellow while rendering the prompt label in the
   same cyan/BOLD tone used elsewhere in the restored wizard chrome.
3. Add focused RED/GREEN coverage proving the input steps no longer render
   nested titled blocks inside the popup body.

### Phase 13: Old-TUI Single-Surface Popup Content

1. Remove the remaining nested content borders from generic option-list steps
   and specialized list renderers so popup chrome remains the only boxed
   surface throughout the wizard.
2. Keep old-TUI row formatting, `VersionSelect` overflow indicators, and AI
   suggestion loading/error copy while dropping the duplicate inner titles.
3. Add focused RED/GREEN coverage for generic lists, model/version steps, and
   AI suggestion loading state to prevent the double-box regression.

## Dependencies

- `reqwest` or `ureq` crate for HTTP client (npm registry fetch).
- `tokio` runtime (already in use) for async cache refresh.
- `serde_json` for cache file serialization (already a dependency).
