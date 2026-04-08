# Quickstart: SPEC-9 - Infrastructure

## Reviewer Flow
1. Run the current infrastructure flows that are reachable from the product shell or CLI.
2. Verify hooks merge behavior against backup and restore expectations first.
3. Inspect Docker-related UI screens and confirm the background lifecycle worker drives DockerProgress transitions.
4. Confirm builtin embedded skills are present at startup and can be toggled from Settings.
5. Run the focused ServiceSelect, PortSelect, DockerProgress, and container lifecycle checks before treating the Docker UI slice as advanced.
6. Track the remaining release packaging gaps as open execution work.

## Repeatable Evidence
- `cargo test -p gwt-skills -- --nocapture`
- `cargo test -p gwt-tui model_new_defaults -- --nocapture`
- `cargo test -p gwt-tui docker_progress -- --nocapture`
- `cargo test -p gwt-tui update_branches_docker_stop_executes_and_refreshes_detail -- --nocapture`
- `cargo test -p gwt-tui update_branches_docker_restart_failure_routes_error_notification -- --nocapture`
- `cargo test -p gwt-tui settings -- --nocapture`
- `cargo test -p gwt-tui service_select -- --nocapture`
- `cargo test -p gwt-tui port_select -- --nocapture`
- `cargo test -p gwt-git -- --nocapture`
- `cargo test -p gwt-skills set_enabled -- --nocapture`
- `cargo test -p gwt-docker --lib -- --nocapture`

## US-9 / US-10 Focused Evidence

Run these after the Phase 2b.6 implementation lands:

- `cargo test -p gwt-skills settings_local -- --nocapture`
- `cargo test -p gwt-skills node_runtime_hook_command_is_byte_identical_across_platforms -- --nocapture`
- `cargo test -p gwt-skills pretooluse_bash_blockers_match_spec_order -- --nocapture`
- `cargo test -p gwt-skills contains_legacy_runtime_shell_command -- --nocapture`
- `cargo test -p gwt-skills migration_replaces_posix_shell_runtime_with_node_form -- --nocapture`
- `cargo test -p gwt-skills migration_replaces_powershell_runtime_with_node_form -- --nocapture`
- `cargo test -p gwt-skills distribute_to_worktree_does_not_write_gwt_forward_hook -- --nocapture`
- `cargo test -p gwt-skills gwt_runtime_state_mjs_writes_sidecar_atomically -- --nocapture` (integration: spawns `node` on the bundled script in a temp dir)
- Manual smoke: in a scratch worktree, run
  - `node .claude/hooks/scripts/gwt-block-cd-command.mjs <<<'{"tool_input":{"command":"cd /tmp"}}'` → exit code `2` with JSON body
  - `node .claude/hooks/scripts/gwt-block-cd-command.mjs <<<'{"tool_input":{"command":"cd ./src"}}'` → exit code `0`
  - `node .claude/hooks/scripts/gwt-block-file-ops.mjs <<<'{"tool_input":{"command":"rm -rf ../outside"}}'` → exit `2`
  - `node .claude/hooks/scripts/gwt-block-git-branch-ops.mjs <<<'{"tool_input":{"command":"git checkout main"}}'` → exit `2`
  - `node .claude/hooks/scripts/gwt-block-git-branch-ops.mjs <<<'{"tool_input":{"command":"git branch --show-current"}}'` → exit `0`
  - `node .claude/hooks/scripts/gwt-block-git-dir-override.mjs <<<'{"tool_input":{"command":"GIT_DIR=/tmp/repo git status"}}'` → exit `2`
  - `GWT_SESSION_RUNTIME_PATH=/tmp/gwt-sidecar.json node .claude/hooks/scripts/gwt-runtime-state.mjs SessionStart` → exit `0`; `cat /tmp/gwt-sidecar.json` shows `{status, updated_at, last_activity_at, source_event}`.
- Cross-platform equivalence check: compare the generated `command` string for `SessionStart` from a POSIX host test run against a Windows host test run; require byte-identical output modulo worktree path.
- Migration smoke: drop a hand-crafted tracked `.codex/hooks.json` that contains `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'`, launch an agent, and verify the file is migrated to `node .../gwt-runtime-state.mjs` while any user hooks remain untouched.

## Expected Result
- The reviewer sees the current implemented scope for infrastructure.
- Any remaining gaps are reviewer-flow or end-to-end acceptance items, not unchecked implementation tasks.
- No step should be treated as complete unless the code path is actually reachable today.
