# TDD Checklist: SPEC-2 - Workspace Shell

Use this checklist to reconcile test-first execution against the SPEC scope.
- [ ] RED: Help overlay tests prove all registered bindings are surfaced and stale bindings are excluded.
- [ ] RED: Session persistence round-trip and corrupted-file fallback tests fail before implementation.
- [ ] RED: Branch detail action tests fail before Launch Agent and Open Shell integration is wired.
- [ ] GREEN: `cargo test -p gwt-tui` and `cargo test -p gwt-tui --test snapshot_e2e` pass together.
- [ ] REFACTOR: Management-tab and branch-detail routing stays simple after tests are green.
