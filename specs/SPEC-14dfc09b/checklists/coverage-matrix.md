## Feature Flow Coverage Matrix

Coverage target: 90% — Achieved: **100%** (24/24 flows)

| # | Domain | Flow | start | success | failure | Status |
|---|--------|------|:-----:|:-------:|:-------:|--------|
| FF-001 | startup | app_init | ✅ | ✅ | — | covered |
| FF-002 | project | open_project | ✅ | ✅ | ✅ | covered |
| FF-003 | project | close_project | ✅ | ✅ | — | covered |
| FF-004 | project | create_project | ✅ | ✅ | ✅ | covered |
| FF-005 | project | start_migration_job | ✅ | ✅ | ✅ | covered |
| FF-006 | config | get_settings | ✅ | ✅ | ✅ | covered |
| FF-007 | config | save_settings | ✅ | ✅ | ✅ | covered |
| FF-008 | issue | fetch_github_issues | ✅ | ✅ | ✅ | covered |
| FF-009 | report | read_recent_logs | ✅ | ✅ | ✅ | covered |
| FF-010 | assistant | assistant_start | ✅ | ✅ | ✅ | covered |
| FF-011 | assistant | assistant_stop | ✅ | ✅ | — | covered |
| FF-012 | assistant | assistant_send_message | ✅ | ✅ | ✅ | covered |
| FF-013 | worktree | list | ✅ | ✅ | — | covered |
| FF-014 | worktree | create_for_branch | ✅ | ✅ | — | covered |
| FF-015 | worktree | create_new_branch | ✅ | ✅ | — | covered |
| FF-016 | worktree | remove | ✅ | ✅ | — | covered |
| FF-017 | git | branch_create | ✅ | ✅ | ✅ | covered |
| FF-018 | git | branch_delete | ✅ | ✅ | ✅ | covered |
| FF-019 | terminal | launch_terminal | ✅ | ✅ | ✅ | covered |
| FF-020 | terminal | launch_agent | ✅ | ✅ | ✅ | covered |
| FF-021 | terminal | close_terminal | ✅ | ✅ | ✅ | covered |
| FF-022 | docker | start | ✅ | ✅ | ✅ | covered |
| FF-023 | docker | stop | ✅ | ✅ | ✅ | covered |
| FF-024 | migration | execute_migration | ✅ | ✅ | ✅ | covered |

Domains covered: startup, project, config, issue/report, assistant, worktree, git, terminal, docker, migration (10/10)

## Incident Response Coverage Matrix

Coverage target: 90% — Achieved: **100%** (27/27 scenarios)

| # | Domain | Scenario | category | event | context | error | Status |
|---|--------|----------|:--------:|:-----:|:-------:|:-----:|--------|
| IR-001 | project | path_not_found | ✅ | ✅ | ✅ | ✅ | covered |
| IR-002 | project | repo_resolve_fail | ✅ | ✅ | ✅ | ✅ | covered |
| IR-003 | project | migration_required | ✅ | ✅ | ✅ | ✅ | covered |
| IR-004 | project | clone_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-005 | terminal | no_project | ✅ | ✅ | ✅ | ✅ | covered |
| IR-006 | terminal | launch_config_error | ✅ | ✅ | ✅ | ✅ | covered |
| IR-007 | assistant | no_project | ✅ | ✅ | ✅ | ✅ | covered |
| IR-008 | assistant | context_resolve_fail | ✅ | ✅ | ✅ | ✅ | covered |
| IR-009 | assistant | analysis_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-010 | git | repo_not_found | ✅ | ✅ | ✅ | ✅ | covered |
| IR-011 | git | pull_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-012 | git | fetch_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-013 | git | worktree_remove_fail | ✅ | ✅ | ✅ | ✅ | covered |
| IR-014 | worktree | branch_already_exists | ✅ | ✅ | ✅ | ✅ | covered |
| IR-015 | worktree | uncommitted_changes | ✅ | ✅ | ✅ | ✅ | covered |
| IR-016 | worktree | reset_failed | ✅ | ✅ | ✅ | ✅ | covered |
| IR-017 | docker | compose_up_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-018 | docker | compose_down_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-019 | docker | compose_build_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-020 | docker | exec_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-021 | migration | validation_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-022 | migration | bare_clone_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-023 | migration | worktree_add_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-024 | config | load_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-025 | config | save_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-026 | config | parse_failure | ✅ | ✅ | ✅ | ✅ | covered |
| IR-027 | system | system_info_join_error | ✅ | ✅ | ✅ | ✅ | covered |

## Known Gaps (deferred)

- `TODO(#1758)`: `error_detail` may contain absolute file paths revealing the local username. Frontend `privacyMask.ts` covers API keys/tokens but not home-directory paths. Low severity.
