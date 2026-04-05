# Quickstart: SPEC-3 - Agent Management

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and open the agent launch or conversion flow from the current session.
2. Verify existing-branch launches start at branch action and spec-prefilled
   launches start at branch type selection before issue and AI naming.
3. Verify built-in detection, config-backed custom agent listing from
   `[tools.customCodingAgents]` in `~/.gwt/config.toml`, and the dedicated
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
   verify `Quick Start` shows the compact branch-name context line,
   agent-labeled action rows, `Resume`, `Start new`, and
   `Choose different`.
8. Launch from an existing branch without persisted session history and
   verify the wizard starts at `BranchAction` instead of showing a stub
   Quick Start placeholder.
9. Choose `Resume` for an entry with a persisted resume session ID and
   confirm the launch args restore resume mode. If no resume session ID
   exists, confirm the path falls back to `Continue`.
10. Choose `Start new` and confirm the wizard keeps the previous
   model/reasoning/version/permissions while resetting session continuity.
11. Launch the session from `Skip Permissions` and confirm a new agent tab
   appears with persisted session metadata while a default model label does
   not become a literal CLI override.
12. Open `Settings > Custom Agents`, add a new custom agent, and verify the
   new entry persists immediately in `~/.gwt/config.toml`.
13. Edit that custom agent's display name, type, and command in Settings,
   then confirm it appears after the built-in agents and launches with the
   configured display name, runner, mode args, and env vars.
14. Change the custom agent command to an invalid binary and confirm the
   launch error mentions the failing command.
15. Trigger session conversion and confirm the active session metadata changes
   while repository context is preserved.
16. Check the existing focused tests and notifications to confirm the original
   session remains intact on conversion failure.
17. Verify the wizard popup border uses the current step title and shows a
   right-aligned `[ESC]` hint.
18. Verify `AgentSelect` from an existing branch shows the compact
   branch-name line above a name-only agent list with the old-TUI cyan
   selection highlight.
19. Verify `BranchNameInput` and `IssueSelect` render inline prompt labels
   inside the popup body and do not add nested titled boxes.
20. Verify list-oriented steps such as `BranchAction`, `ModelSelect`,
   `VersionSelect`, and AI suggestion loading/error use the popup chrome as
   the only box while keeping their row formatting and copy visible.
21. Verify the AI suggestion candidate list also keeps `Context: ...`
   visible above the suggestions and `Manual input`.
22. Verify AI suggestion loading and error states render `Context: ...` as a
   standalone row above the body copy, matching the candidate-list layout.
23. Verify AI suggestion loading and error body copy stays compact and does
   not repeat manual-input guidance that already appears in the footer hint
   row.
24. Verify generic option lists such as `BranchAction` and `ModelSelect` use
   the same cyan selected-row highlight as `QuickStart` and `AgentSelect`.
25. Verify `BranchNameInput` and `IssueSelect` now render the prompt and the
   yellow input value on separate rows instead of sharing a single inline
   paragraph.
26. Verify `QuickStart` now starts the first grouped history entry directly
   below the compact branch-name context line instead of leaving an extra
   blank row before the grouped list.
27. Verify consecutive Quick Start agent groups now render without a blank
   spacer row between them while `Choose different` still follows
   the grouped history as the final footer action.
28. Verify `Choose different` now appears directly below the last
   grouped `Start new` row without an extra separator line.
29. Verify the final Quick Start action now uses the label-only
   `Choose different` copy on wide and narrow popups.
30. Verify the final action label no longer uses an ellipsis and now matches
   the old-TUI copy `Choose different`.
31. Verify grouped Quick Start actions now read `Resume` / `Start new`,
   while resume-capable entries still show the short session ID snippet.
32. Verify a branch with exactly one persisted Quick Start entry now shows
   `Quick Start — Agent (Model)` in the popup title and starts the action
   rows immediately below the compact branch-name context line without a
   duplicated grouped header row.
33. Verify a branch with multiple persisted Quick Start entries now renders
   action rows where only the `Resume` row keeps the inline agent label
   (`Codex Resume`, `Claude Code Resume`), while the paired `Start new` rows
   stay plain and compact.
34. Verify those plain multi-entry `Start new` rows now render in neutral
   text color, so the colored agent identity remains on the paired
   `Resume` rows only.
35. Verify a branch with multiple persisted Quick Start entries now shows the
   short resume-session ID snippet only on the selected `Resume` row, while
   unselected resume rows keep the plain label.
36. Verify a branch with multiple persisted Quick Start entries now renders
   grouped action rows as `Resume` / `Start new`, while a single-entry Quick
   Start also uses the compact `Resume` / `Start new` copy.
37. Verify the final Quick Start action now stays `Choose different`
   on both wide and narrow popups, without the rebuilt `Open full setup`
   description text.
38. Verify a branch with multiple persisted Quick Start entries keeps the
   compact `Resume` / `Start new` labels while moving selection, and a
   single-entry Quick Start keeps the same compact action copy.
39. Verify the wizard popup no longer shows a separate `Step N/M` row above
   the chrome and still keeps the step title in the border.
40. Verify a single-entry Quick Start with no persisted model now promotes
   only the agent label into the popup title (`Quick Start — Codex`) instead
   of inventing a `default` model placeholder.

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
- `cargo test -p gwt-tui load_custom_agents_from_path_parses_spec_schema -- --nocapture`
- `cargo test -p gwt-tui save_stored_custom_agents_to_path_preserves_models_and_other_settings -- --nocapture`
- `cargo test -p gwt-tui build_wizard_agent_options_with_custom_agents_appends_settings_agents -- --nocapture`
- `cargo test -p gwt-tui build_launch_config_from_wizard_with_custom_agents_uses_custom_command_and_display_name -- --nocapture`
- `cargo test -p gwt-tui custom_agents_category_loads_persisted_agent_fields -- --nocapture`
- `cargo test -p gwt-tui custom_agents_add_edit_delete_persist_immediately -- --nocapture`
- `cargo test -p gwt-tui materialize_pending_launch_with -- --nocapture`
- `cargo test -p gwt-tui session_conversion`

## Expected Result
- The reviewer sees the current implemented scope for agent management.
- Version selection is visibly independent from model selection and matches
  the launch path without a trailing confirm screen.
- Quick Start behaves like the old TUI for persisted branch history instead of
  acting as a static placeholder.
- Settings > Custom Agents now supports add/edit/delete directly in the TUI,
  persists each change immediately, and preserves unrelated config sections.
- Config-backed custom agents now appear in `AgentSelect` after the built-in
  entries and launch with their configured runner, display name, mode args,
  and env vars.
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
- The final `Choose different` action now follows the last grouped
  row directly instead of being separated by its own divider.
- The grouped Quick Start actions now use the shorter old-TUI labels
  `Resume` and `Start new` while keeping the short resume-session hint where
  available.
- Single-entry Quick Start popups now move the lone agent/model summary into
  the popup title so the body can start directly with the available actions
  under the compact branch-name context line.
- Multi-entry Quick Start popups now inline the agent label into each action
  row so the grouped list remains denser and the extra model/reasoning detail
  does not repeat across every group.
- Multi-entry Quick Start resume rows now reserve the short session ID hint
  for the currently selected row, so the grouped history stays visually
  quieter when several tools can resume.
- Multi-entry Quick Start action rows now use the denser old-TUI copy
  `Resume` / `Start new`, and single-entry Quick Start now matches the same
  compact action copy.
- The final Quick Start action now stays label-only on both wide and narrow
  popups, so the footer no longer uses the rebuilt inline description text.
- Quick Start state-derived option labels now match the rendered grouped rows,
  so both multi-entry and single-entry history stay on compact `Resume` /
  `Start new` copy.
- The popup no longer shows a separate `Step N/M` row above the chrome, so
  the border title remains the only step-context chrome.
- Quick Start now renders the branch name as a compact context line instead
  of the rebuilt `Branch: ...` copy, while keeping the grouped history order
  unchanged.
- Single-entry Quick Start title promotion now keeps `Agent (Model)` only,
  dropping the rebuilt reasoning copy while leaving body actions unchanged.
- Multi-entry Quick Start now inlines agent labels into each action row
  instead of rendering standalone grouped headers, keeping grouped history
  dense while preserving compact `Resume` / `Start new` copy.
- Single-entry Quick Start now falls back to the bare agent label when no
  model was persisted, instead of showing a synthetic `default` model in the
  popup title.
- The final Quick Start action now uses the compact old-TUI copy
  `Choose different`, and `current_options()` now returns the same shorter
  footer label.
- Existing-branch AgentSelect now uses that same compact branch-name line and
  places the first agent row directly below it instead of leaving an extra
  spacer row after a `Branch: ...` prefix.
- Multi-entry Quick Start now indents the plain `Start new` rows beneath the
  paired `Resume` rows so the old-TUI primary/secondary action hierarchy is
  visible again without adding standalone headers back.
- Any missing behavior is logged against acceptance or reviewer gaps rather than unchecked implementation tasks.
- No step should be treated as complete unless the code path is actually reachable today.
