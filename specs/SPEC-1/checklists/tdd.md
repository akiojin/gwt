# TDD Checklist: SPEC-1 - Terminal Emulation

Use this checklist to reconcile test-first execution against the SPEC scope.
- [ ] RED: URL pattern matching and coordinate-region tests exist for single, multiple, and wrapped URLs.
- [ ] RED: Ctrl+click opener tests fail before the browser integration is wired.
- [ ] RED: Alt-screen round-trip fixtures fail before verification logic is added.
- [ ] GREEN: `cargo test -p gwt-tui` passes after the URL and alt-screen work is complete.
- [ ] REFACTOR: Shared URL detection or opener logic is extracted only after tests stay green.
