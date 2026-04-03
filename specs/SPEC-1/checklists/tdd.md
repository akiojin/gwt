# TDD Checklist: SPEC-1 - Terminal Emulation

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Repository-level regression checks exist, and feature-focused terminal tests are now tied to the delivered tasks.
- [x] Each remaining unchecked task has a focused failing test or repeatable manual check defined.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence.
