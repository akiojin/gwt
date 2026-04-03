# TDD Checklist: SPEC-3 - Agent Management

Use this checklist to reconcile test-first execution against the SPEC scope.
- [ ] RED: Version cache expiry, corruption, and fallback tests exist and stay authoritative.
- [ ] RED: Session conversion success tests fail until working-directory preservation is implemented.
- [ ] RED: Session conversion failure tests fail until the original session is restored cleanly.
- [ ] GREEN: `cargo test -p gwt-tui` covers startup cache flow and session-conversion paths.
- [ ] REFACTOR: Wizard startup and conversion helpers stay separated from overlay rendering concerns.
