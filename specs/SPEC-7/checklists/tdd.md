# TDD Checklist: SPEC-7 - Settings and Profiles

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Broad repo checks pass, but SPEC-7 still needs focused form-state and validation coverage.
- [ ] Each remaining unchecked task has a focused failing test or repeatable manual check defined.
- [ ] The latest implementation slice has spec-focused verification evidence attached to it.
- [ ] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence.
