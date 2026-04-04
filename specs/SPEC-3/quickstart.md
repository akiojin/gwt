# Quickstart: SPEC-3 - Agent Management

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and open the agent launch or conversion flow from the current session.
2. Verify existing-branch launches start at branch action and spec-prefilled
   launches start at branch type selection before issue and AI naming.
3. Verify built-in detection, custom agent listing, and the dedicated
   VersionSelect step in the wizard.
4. For an npm-backed agent, confirm the version list shows the installed
   runner, `latest`, and cached semver entries without duplication.
5. Verify existing-branch launches now start at `BranchAction`, and for
   Codex the flow includes `Model -> Reasoning -> Version -> Execution Mode
   -> Skip Permissions` without a trailing confirm screen.
6. Verify `ModelSelect`, `ReasoningLevel`, `ExecutionMode`, and
   `SkipPermissions` render descriptive old-TUI rows and `VersionSelect`
   shows scroll indicators when the list overflows.
7. Launch from an existing branch that has persisted session history and
   verify `Quick Start` shows `Branch: ...`, grouped agent headers,
   `Resume`, `Start new`, and `Choose different settings`.
8. Launch from an existing branch without persisted session history and
   verify the wizard starts at `BranchAction` instead of showing a stub
   Quick Start placeholder.
9. Choose `Resume` for an entry with a persisted resume session ID and
   confirm the launch args restore resume mode. If no resume session ID
   exists, confirm the path falls back to `Continue`.
10. Choose `Start new with previous settings` and confirm the wizard keeps the
   previous model/reasoning/version/permissions while resetting session
   continuity.
11. Launch the session from `Skip Permissions` and confirm a new agent tab
   appears with persisted session metadata while a default model label does
   not become a literal CLI override.
12. Trigger session conversion and confirm the active session metadata changes
   while repository context is preserved.
13. Check the existing focused tests and notifications to confirm the original
   session remains intact on conversion failure.
14. Verify the wizard popup border uses the current step title and shows a
   right-aligned `[ESC]` hint.
15. Verify `AgentSelect` from an existing branch shows `Branch: ...` above a
   name-only agent list with the old-TUI cyan selection highlight.
16. Verify `BranchNameInput` and `IssueSelect` render inline prompt labels
   inside the popup body and do not add nested titled boxes.
17. Verify list-oriented steps such as `BranchAction`, `ModelSelect`,
   `VersionSelect`, and AI suggestion loading/error use the popup chrome as
   the only box while keeping their row formatting and copy visible.
18. Verify the AI suggestion candidate list also keeps `Context: ...`
   visible above the suggestions and `Manual input`.
19. Verify AI suggestion loading and error states render `Context: ...` as a
   standalone row above the body copy, matching the candidate-list layout.
20. Verify AI suggestion loading and error body copy stays compact and does
   not repeat manual-input guidance that already appears in the footer hint
   row.
21. Verify generic option lists such as `BranchAction` and `ModelSelect` use
   the same cyan selected-row highlight as `QuickStart` and `AgentSelect`.
22. Verify `BranchNameInput` and `IssueSelect` now render the prompt and the
   yellow input value on separate rows instead of sharing a single inline
   paragraph.
23. Verify `QuickStart` now starts the first grouped history entry directly
   below the `Branch: ...` context line instead of leaving an extra blank row
   before the grouped list.
24. Verify consecutive Quick Start agent groups now render without a blank
   spacer row between them while `Choose different settings` still follows
   the grouped history as the final footer action.
25. Verify `Choose different settings` now appears directly below the last
   grouped `Start new` row without an extra separator line.
26. Verify wide popups now render `Choose different settings - Open full
   setup` while narrow popups fall back to the label-only row.
27. Verify the final action label no longer uses an ellipsis and now matches
   the old-TUI copy `Choose different settings`.
28. Verify grouped Quick Start actions now read `Resume session` / `Start new
   session`, while resume-capable entries still show the short session ID
   snippet.
29. Verify a branch with exactly one persisted Quick Start entry now shows
   `Quick Start — <Agent/Model>` in the popup title and starts the action
   rows immediately below `Branch: ...` without a duplicated grouped header
   row.

## Repeatable Evidence
- `cargo test -p gwt-agent detect -- --nocapture`
- `cargo test -p gwt-agent version_cache -- --nocapture`
- `cargo test -p gwt-tui wizard -- --nocapture`
- `cargo test -p gwt-tui render_ -- --nocapture`
- `cargo test -p gwt-tui render_agent_select -- --nocapture`
- `cargo test -p gwt-tui render_popup_chrome_shows_step_title_and_esc_hint -- --nocapture`
- `cargo test -p gwt-tui render_agent_select_for_existing_branch_shows_branch_and_name_only_rows -- --nocapture`
- `cargo test -p gwt-tui render_agent_select_uses_old_tui_selection_and_agent_colors -- --nocapture`
- `cargo test -p gwt-tui quick_start -- --nocapture`
- `cargo test -p gwt-tui prepare_wizard_startup_starts_spec_prefill_at_branch_type_select -- --nocapture`
- `cargo test -p gwt-tui build_launch_config_from_wizard -- --nocapture`
- `cargo test -p gwt-tui materialize_pending_launch_with -- --nocapture`
- `cargo test -p gwt-tui session_conversion`

## Expected Result
- The reviewer sees the current implemented scope for agent management.
- Version selection is visibly independent from model selection and matches
  the launch path without a trailing confirm screen.
- Quick Start behaves like the old TUI for persisted branch history instead of
  acting as a static placeholder.
- AgentSelect and popup chrome now match the old-TUI visual contract for the
  restored branch-first flow.
- Branch and issue input steps now follow that same contract with inline
  prompt labels instead of nested titled boxes.
- List-oriented wizard steps now follow the same contract, so popup chrome is
  the only boxed surface even during model/version selection and AI loading.
- The AI suggestion step now keeps its `Context: ...` line visible in every
  state instead of dropping it when suggestions arrive.
- The AI suggestion loading and error states now render that context as the
  same standalone row used by the candidate list instead of embedding it in
  the paragraph copy.
- The AI suggestion loading and error body copy now stays compact, and the
  footer hint row remains the single source of manual-input guidance.
- Wizard list-based steps now share the same cyan selected-row highlight
  instead of mixing dark-gray generic selection with specialized cyan rows.
- Branch and issue input steps now match that same compact old-TUI rhythm by
  splitting the prompt and value across two rows instead of compressing both
  into one line.
- Quick Start now matches that denser popup rhythm as well, with grouped
  history rows starting immediately below the branch context line.
- Quick Start agent groups now render back-to-back without spacer rows,
  making the grouped history denser while preserving the final footer action.
- The final `Choose different settings` action now follows the last grouped
  row directly instead of being separated by its own divider.
- The grouped Quick Start actions now use the shorter old-TUI labels
  `Resume session` and `Start new session` while keeping the short
  resume-session hint where available.
- The final `Choose different settings` row now explains itself on wide
  popups via `label - description` formatting and keeps the old label-only
  fallback on narrow widths.
- Single-entry Quick Start popups now move the lone agent/model summary into
  the popup title so the body can start directly with the available actions
  under `Branch: ...`.
- Any missing behavior is logged against acceptance or reviewer gaps rather than unchecked implementation tasks.
- No step should be treated as complete unless the code path is actually reachable today.
