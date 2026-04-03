# TDD Checklist: SPEC-9 - Infrastructure

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Broad repo checks pass, but SPEC-9 still needs focused infrastructure verification and manual release-style checks.
- [ ] Each remaining unchecked task has a focused failing test or repeatable manual check defined.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [ ] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence.
