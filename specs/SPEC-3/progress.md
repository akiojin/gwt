# Progress: SPEC-3 - Agent Management

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `185/185` checked in `tasks.md`
- Artifact refresh: `2026-04-07T01:45:00Z`

## Done
- Startup cache scheduling, wizard integration, and session conversion flow documentation are now aligned to the implemented code.
- Session conversion artifacts now consistently describe the implemented metadata-driven agent switch instead of PTY relaunch.
- Supporting artifacts now cover execution, review, and completion-gate reconciliation for this near-finished SPEC.
- Issue detail launches now rejoin the standard new-branch wizard flow, so
  Branches, SPEC detail, and Issue detail all enter `BranchType -> Issue ->
  Branch Name` without requiring AI configuration.
- Wizard version selection is now a dedicated step, with focused tests for
  installed-version fallback, cache-backed options, and confirm-summary
  rendering.
- Launch confirmation now materializes a persisted agent session after the
  wizard closes, with focused tests for config normalization and session-file
  creation.
- Wizard launch now follows a branch-first flow again: existing-branch
  launches begin at branch action, while spec-prefilled launches begin at
  branch type selection before issue and manual branch input, with the AI
  suggestion step left dormant in the standard flow.
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
- SkipPermissions now restores legacy flags per agent (Claude:
  `--dangerously-skip-permissions`; Codex/Gemini/Copilot: `--yolo`) and custom
  agents can supply `skipPermissionsArgs` for opt-in behavior.
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
- The AI suggestion candidate list now keeps `Context: ...` visible after
  loading finishes, so all AI suggestion states share the same context-first
  old-TUI content contract.
- AI suggestion loading and error states now render that `Context: ...` line
  as a standalone cyan row above the body copy instead of embedding it inline
  inside the paragraph text.
- AI suggestion loading and error body copy now stays compact and leaves
  manual-input guidance to the footer hint row instead of repeating it in the
  popup body.
- Wizard list-based steps now share the same cyan selected-row highlight,
  so generic option lists and specialized steps no longer drift visually.
- `BranchNameInput` and `IssueSelect` now use the old-TUI compact two-row
  layout, with a cyan prompt line above the yellow input value while the
  popup chrome remains the only boxed surface.
- `QuickStart` now begins its grouped history immediately below the compact
  branch-name context line instead of reserving an extra spacer row, which
  restores the old-TUI popup density.
- `QuickStart` agent groups now render back-to-back without blank spacer rows
  between them, preserving headers while matching the denser old-TUI grouped
  layout.
- The final `Choose different` action now follows the last grouped
  `Start new` row directly without an extra separator line, completing the
  denser old-TUI footer rhythm.
- `QuickStart` action rows now use the shorter old-TUI labels `Resume` and
  `Start new`, while still showing a resume session ID snippet when one
  exists.
- The final action label now matches the old-TUI copy `Choose different`
  without an ellipsis.
- Single-entry `QuickStart` popups now promote the lone agent/model summary
  into the popup title and start the action rows directly below
  the compact branch-name context line, while multi-entry history keeps the
  generic `Quick Start` title.
- Multi-entry `QuickStart` now inlines agent labels into each action row,
  leaving the more detailed model/reasoning summary to the single-entry title
  variant and keeping grouped history visually denser.
- Multi-entry `QuickStart` now shows the short resume-session ID hint only on
  the selected `Resume` row, which reduces noise on unselected rows without
  changing resume semantics.
- Multi-entry `QuickStart` grouped action rows now use the denser old-TUI
  copy `Resume` / `Start new`.
- The final `QuickStart` action now stays label-only (`Choose different`) on
  both wide and narrow popups, removing the rebuilt inline
  description text.
- `QuickStart` now keeps its state-derived option labels aligned with the
  rendered grouped rows, so both multi-entry and single-entry history now use
  compact `Resume` / `Start new` copy consistently.
- The wizard popup now uses the border title as its only step-context chrome,
  removing the redundant `Step N/M` row so content starts one line higher.
- `QuickStart` now uses a compact branch-name context line instead of the
  rebuilt `Branch: ...` prefix, which tightens the popup body without
  changing grouped history ordering.
- Single-entry `QuickStart` title promotion now keeps `Agent (Model)` only,
  dropping the rebuilt reasoning copy while preserving compact body actions.
- Multi-entry `QuickStart` now inlines the agent label into each action row,
  so grouped history no longer depends on standalone header rows while
  keeping the compact `Resume` / `Start new` copy and selected resume hint.
- Single-entry `QuickStart` title promotion now falls back to the bare agent
  label when no model was persisted, instead of synthesizing a `default`
  placeholder into the popup title.
- The final `QuickStart` action label is now compacted from
  `Choose different settings` to `Choose different`, while preserving the
  same selection semantics and footer density.
- Multi-entry `QuickStart` now keeps the agent label on the `Resume` row
  only, while the paired `Start new` row falls back to the compact plain
  label.
- Multi-entry `QuickStart` now renders those plain `Start new` rows in a
  neutral color, leaving the inline `Resume` row as the only agent-colored
  identity row in each entry block.
- Multi-entry `QuickStart` now indents those plain `Start new` rows beneath
  the paired `Resume` row, restoring the old-TUI primary/secondary action
  hierarchy without reintroducing standalone headers.
- Existing-branch `AgentSelect` now uses the same compact branch-name line as
  `QuickStart`, and the first agent row starts directly below that context
  instead of after an extra spacer row.
- Wizard startup now reads valid `[tools.customCodingAgents.*]` entries from
  `~/.gwt/config.toml`, appends them after the built-in AgentSelect entries,
  and builds launches from the configured command/path/bunx runner plus mode
  args and env vars.
- Settings > Custom Agents now loads persisted entries from
  `~/.gwt/config.toml`, supports add/edit/delete directly in the TUI, and
  persists each change immediately without a separate save step.
- Shared custom-agent config helpers now preserve unrelated settings and
  unknown nested custom-agent sections such as `models`, so Settings edits do
  not erase GUI-era configuration value.
- Launch Agent's Codex model list now matches the current Codex CLI snapshot,
  including `gpt-5.4`, `gpt-5.4-mini`, and `gpt-5.3-codex-spark`, while
  keeping `Default (Auto)` on the no-`--model` path.
- New-branch launches now materialize a sibling git worktree before PTY
  spawn, so Launch Agent no longer falls back to the repository root
  checkout when creating a new branch.
- Branch-origin launches now preserve the selected base branch for
  worktree creation, while SPEC/Issue-prefilled launches default that base
  branch to `develop`.
- Selected-branch new-branch launches no longer leak the selected branch's
  existing worktree into `LaunchConfig`, so launch-time materialization now
  still creates the requested sibling worktree instead of silently reusing
  `develop`.
- Launches started from a linked worktree such as `develop` now resolve the
  main repository root before deriving the sibling layout, so new branches no
  longer materialize under `develop-*` paths like `develop-feature-test`.
- Launches started from a legacy bare workspace layout (`gwt.git` +
  `develop/`) now use the bare common-dir as the Git control path while
  stripping the `.git` suffix from the repo name for sibling path derivation,
  so new branches no longer materialize under `develop-*` paths like
  `develop-feature-test2`.
- If a branch was already materialized into a stale worktree path from an
  earlier launch bug, Launch Agent now reuses that existing branch worktree
  instead of failing a second `git worktree add`.
- The actual launched worktree path is now persisted into session metadata
  and exported through `GWT_PROJECT_ROOT`, so Quick Start / resume operate on
  the materialized launch target instead of a repo-root alias.
- Verification for the Codex model sync slice now includes `cargo test -p
  gwt-tui`, `cargo test -p gwt-core -p gwt-tui`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo build -p gwt-tui`, and `bunx markdownlint-cli2` on the refreshed
  SPEC-3 artifacts.
- Focused verification for the worktree-materialization slice now includes
  `cargo test -p gwt-git sibling_worktree_path_uses_repo_name_and_slugged_branch -- --nocapture`,
  `cargo test -p gwt-git create_from_base_creates_new_branch_worktree -- --nocapture`,
  `cargo test -p gwt-git main_worktree_root_returns_primary_repo_for_linked_worktree -- --nocapture`,
  `cargo test -p gwt-git bare_common_dir -- --nocapture`,
  `cargo test -p gwt-tui base_branch -- --nocapture`, and
  `cargo test -p gwt-tui materialize_pending_launch_with_new_branch_creates_worktree_and_persists_actual_path -- --nocapture`,
  `cargo test -p gwt-tui linked_worktree_uses_main_repo_sibling_layout -- --nocapture`,
  `cargo test -p gwt-tui bare_workspace_linked_worktree_uses_repo_name_layout -- --nocapture`,
  `cargo test -p gwt-tui existing_branch_worktree_reuses_previous_path -- --nocapture`,
  plus `cargo test -p gwt-tui from_selected_branch -- --nocapture`.

## Next
- Run the manual reviewer flow in `quickstart.md` and close the remaining
  acceptance checklist items for the restored wizard and custom-agent UX.
