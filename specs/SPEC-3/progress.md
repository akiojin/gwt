# Progress: SPEC-3 - Agent Management

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `69/69` checked in `tasks.md`
- Artifact refresh: `2026-04-04T11:08:20Z`

## Done
- Startup cache scheduling, wizard integration, and session conversion flow documentation are now aligned to the implemented code.
- Session conversion artifacts now consistently describe the implemented metadata-driven agent switch instead of PTY relaunch.
- Supporting artifacts now cover execution, review, and completion-gate reconciliation for this near-finished SPEC.
- Wizard version selection is now a dedicated step, with focused tests for
  installed-version fallback, cache-backed options, and confirm-summary
  rendering.
- Launch confirmation now materializes a persisted agent session after the
  wizard closes, with focused tests for config normalization and session-file
  creation.
- Wizard launch now follows a branch-first flow again: existing-branch
  launches begin at branch action, while spec-prefilled launches begin at
  branch type selection before issue and AI naming.
- The current ratatui wizard now uses the old-TUI-aligned step machine:
  `BranchAction`, `ConvertAgentSelect`, and `ConvertSessionSelect` are
  restored, and `SkipPermissions` now completes directly without `Confirm`.
- The option-list renderer now restores old-TUI row formatting for
  `ModelSelect`, `ReasoningLevel`, `ExecutionMode`, and `SkipPermissions`,
  and `VersionSelect` now shows descriptive rows plus overflow indicators.
- Existing-branch launches now reconstruct Quick Start history from persisted
  agent sessions, render the old-TUI grouped history layout, and restore
  `Resume` / `Start new` semantics including the resume-ID fallback to
  `Continue`.
- Persisted agent session metadata now stores Quick Start restore fields
  (`reasoning_level`, `skip_permissions`, resume session ID) so future
  launches can replay the previous configuration instead of showing a static
  placeholder.
- Recent verification exists for SPEC-3 slices: `cargo fmt --all`, `cargo test -p gwt-tui`, `cargo test -p gwt-core -p gwt-tui`, `cargo clippy -p gwt-tui --all-targets --all-features -- -D warnings`, `cargo clippy --all-targets --all-features -- -D warnings`, `bunx markdownlint-cli specs/SPEC-3/tasks.md`, and `bunx commitlint --from HEAD~1 --to HEAD`.
- Repeatable reviewer evidence is now captured in `quickstart.md` with detect,
  version-cache, wizard, launch-materialization, and session-conversion test
  commands.
- The popup chrome now restores the old-TUI step-title border with a
  right-aligned `[ESC]` hint, and existing-branch `AgentSelect` now shows
  `Branch: ...` above name-only agent rows with the old-TUI cyan selection
  highlight.
- `BranchNameInput` and `IssueSelect` now match that old-TUI popup contract
  as well, rendering inline cyan prompt labels with yellow input values
  instead of nested titled boxes inside the popup body.
- Generic list steps, `VersionSelect`, and AI suggestion loading/error now
  reuse the popup chrome as the wizard's only boxed surface instead of
  layering redundant inner borders or titles inside the content area.

## Next
- Run the manual reviewer flow in `quickstart.md` and close the remaining
  acceptance checklist items.
