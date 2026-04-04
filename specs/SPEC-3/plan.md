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
   `Choose different settings` option.
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

### Phase 14: AI Suggestion Context Consistency

1. Keep `Context: ...` visible for the AI suggestion step even after the
   loading state transitions into the candidate list.
2. Reuse the popup chrome as the only box while rendering the context line,
   candidate rows, and `Manual input` action in one content surface.
3. Add focused RED/GREEN coverage proving the context survives in the
   suggestion-list state without reintroducing nested content chrome.

### Phase 15: AI Suggestion State Layout Alignment

1. Render `Context: ...` as a standalone line in AI suggestion loading and
   error states so all three states share the same context-first layout.
2. Keep popup chrome as the only boxed surface while the body copy shifts
   downward under the context line.
3. Add focused RED/GREEN coverage for loading and error states to prevent the
   context line from regressing back into inline paragraph copy.

### Phase 16: AI Suggestion Body Copy Compaction

1. Remove duplicate manual-input guidance from AI suggestion loading and error
   body copy now that the footer hint row already carries that instruction.
2. Keep loading/error body text to a single concise line beneath
   `Context: ...` so those states stay visually compact like the old TUI.
3. Add focused RED/GREEN coverage that proves the removed guidance does not
   reappear inside the body content.

### Phase 17: Wizard Selection Highlight Consistency

1. Introduce a wizard-local selected-row style helper so list-based wizard
   steps share the same cyan/black old-TUI selection highlight.
2. Apply that helper to generic option lists, model/version/reasoning rows,
   and the existing `QuickStart` / `AgentSelect` rows without changing step
   transitions or launch semantics.
3. Add focused RED/GREEN coverage proving both generic option lists and
   `ModelSelect` now use the cyan highlight contract.

### Phase 18: Wizard Input Two-Row Layout

1. Split `BranchNameInput` and `IssueSelect` into a cyan prompt row followed
   by a yellow value row so the input steps match the old-TUI vertical rhythm.
2. Keep popup chrome as the only boxed surface and avoid reintroducing titled
   inner boxes or extra separators.
3. Add focused RED/GREEN coverage proving prompt and value render on separate
   rows for both input steps.

### Phase 19: QuickStart Density Restoration

1. Remove the extra spacer row between `Branch: ...` and the first grouped
   Quick Start history entry so the popup matches old-TUI information density.
2. Preserve grouped agent headers, `Resume` / `Start new`, separators, and
   the trailing `Choose different settings` action while tightening only
   the vertical spacing above the list.
3. Add focused RED/GREEN coverage proving the first group begins immediately
   below the branch context line.

### Phase 20: QuickStart Group Density Restoration

1. Remove the blank spacer rows between Quick Start agent groups so the next
   group header follows directly after the previous `Start new` action.
2. Preserve selection-index semantics, colored agent headers, and the final
   separator before `Choose different settings...` while tightening only the
   inter-group spacing.
3. Add focused RED/GREEN coverage proving adjacent groups render without a
   spacer row between them.

### Phase 21: QuickStart Footer Separator Compaction

1. Replace the full-width separator before `Choose different settings`
   with a compact rule so the footer keeps its boundary without dominating the
   popup width.
2. Preserve the final `Choose different settings` action and its
   selection semantics while only lightening the separator chrome.
3. Add focused RED/GREEN coverage proving the footer no longer renders a
   full-width separator rule.

### Phase 22: QuickStart Footer Action Description

1. Render `Choose different settings` as an old-TUI `label - description`
   row on sufficiently wide popups so the final action explains that it opens
   the full setup flow.
2. Preserve selection semantics and narrow-width readability by falling back
   to the existing label-only row when there is not enough space.
3. Add focused RED/GREEN coverage for wide-width description rendering and
   narrow-width fallback behavior.

### Phase 23: QuickStart Footer Separator Removal

1. Remove the remaining footer separator so `Choose different settings`
   follows the last grouped `Start new` action directly in the old-TUI rhythm.
2. Preserve the final action's selection semantics and wide/narrow rendering
   contract while only tightening the vertical density of the grouped history.
3. Add focused RED/GREEN coverage proving the final action now follows the last
   grouped row without an extra separator line.

### Phase 25: QuickStart Final Label Copy

1. Align the final Quick Start action label with the old-TUI copy
   `Choose different settings` by removing the rebuilt ellipsis.
2. Preserve the wide `label - description` row and narrow fallback while only
   changing the label text itself.
3. Add focused RED/GREEN coverage for both wide and narrow render paths using
   the non-ellipsis label.

### Phase 26: QuickStart Single-Entry Title Promotion

1. When `QuickStart` contains exactly one persisted entry, promote that
   agent/model summary into the popup title so the old-TUI chrome carries the
   context instead of repeating it in the body.
2. Preserve multi-entry grouped headers unchanged while omitting the
   duplicated header row only for the single-entry branch, making the first
   action row follow `Branch: ...` directly.
3. Add focused RED/GREEN coverage for single-entry title promotion and
   multi-entry fallback before updating artifacts.

### Phase 27: QuickStart Multi-Entry Header Simplification

1. Keep the single-entry title summary intact, but simplify multi-entry
   grouped headers to the agent label only so the grouped list stays dense
   and action-first.
2. Preserve the generic `Quick Start` title, grouped row ordering, and
   selection semantics while only reducing the header copy for multi-entry
   history.
3. Add focused RED/GREEN coverage proving multi-entry headers no longer show
   model/reasoning details while single-entry title promotion still works.

### Phase 28: QuickStart Selected Resume Hint

1. Keep single-entry `Resume session (sess-123...)` behavior intact, but in
   multi-entry history show the resume-session ID snippet only on the
   selected `Resume session` row.
2. Preserve grouped ordering, labels, and selection semantics while reducing
   visual noise on unselected resume rows.
3. Add focused RED/GREEN coverage proving selected rows keep the short hint
   and unselected rows fall back to the plain `Resume session` label.

### Phase 29: QuickStart Multi-Entry Action Copy Compaction

1. Keep single-entry Quick Start wording intact, but in multi-entry grouped
   history shorten the action rows to the denser old-TUI copy `Resume` /
   `Start new`.
2. Preserve grouped ordering, selected resume-session ID hints, and the final
   `Choose different settings` action while only tightening the grouped body
   copy for multi-entry history.
3. Add focused RED/GREEN coverage proving multi-entry renders the compact
   action labels without affecting single-entry title promotion.

### Phase 24: QuickStart Action Label Restoration

1. Restore the old-TUI action copy so grouped Quick Start rows say
   `Resume session` and `Start new session` instead of the rebuilt
   longer phrases.
2. Preserve resume-session ID hints and existing selection semantics while
   tightening the visual density of the grouped history rows.
3. Add focused RED/GREEN coverage for wide and narrow render paths using the
   restored labels.

## Dependencies

- `reqwest` or `ureq` crate for HTTP client (npm registry fetch).
- `tokio` runtime (already in use) for async cache refresh.
- `serde_json` for cache file serialization (already a dependency).
