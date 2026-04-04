# Progress: SPEC-3 - Agent Management

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `51/51` checked in `tasks.md`
- Artifact refresh: `2026-04-04T02:06:13Z`

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
- Recent verification exists for SPEC-3 slices: `cargo fmt --all`, `cargo test -p gwt-tui`, `cargo test -p gwt-core -p gwt-tui`, `cargo clippy -p gwt-tui --all-targets --all-features -- -D warnings`, `cargo clippy --all-targets --all-features -- -D warnings`, `bunx markdownlint-cli specs/SPEC-3/tasks.md`, and `bunx commitlint --from HEAD~1 --to HEAD`.
- Repeatable reviewer evidence is now captured in `quickstart.md` with detect,
  version-cache, wizard, launch-materialization, and session-conversion test
  commands.

## Next
- Run the manual reviewer flow in `quickstart.md` and close the remaining
  acceptance checklist items.
- Keep Quick Start history presentation restoration separate until the data
  path is wired into the ratatui wizard.
