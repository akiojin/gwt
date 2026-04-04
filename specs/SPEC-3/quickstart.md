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
6. Launch the session from `Skip Permissions` and confirm a new agent tab
   appears with persisted session metadata while a default model label does
   not become a literal CLI override.
7. Trigger session conversion and confirm the active session metadata changes
   while repository context is preserved.
8. Check the existing focused tests and notifications to confirm the original
   session remains intact on conversion failure.

## Repeatable Evidence
- `cargo test -p gwt-agent detect -- --nocapture`
- `cargo test -p gwt-agent version_cache -- --nocapture`
- `cargo test -p gwt-tui wizard -- --nocapture`
- `cargo test -p gwt-tui prepare_wizard_startup_starts_spec_prefill_at_branch_type_select -- --nocapture`
- `cargo test -p gwt-tui build_launch_config_from_wizard -- --nocapture`
- `cargo test -p gwt-tui materialize_pending_launch_with -- --nocapture`
- `cargo test -p gwt-tui session_conversion`

## Expected Result
- The reviewer sees the current implemented scope for agent management.
- Version selection is visibly independent from model selection and matches
  the launch path without a trailing confirm screen.
- Any missing behavior is logged against acceptance or reviewer gaps rather than unchecked implementation tasks.
- No step should be treated as complete unless the code path is actually reachable today.
