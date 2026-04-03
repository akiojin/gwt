# Quickstart: SPEC-1 - Terminal Emulation

## Minimum Validation Flow
1. **Run focused renderer tests** - `cargo test -p gwt-tui renderer::tests -- --nocapture`
2. **Run the TUI locally** - `cargo run -p gwt-tui`
3. **Verify implemented behavior** - Confirm ANSI rendering, scrollback, and drag-selection still behave correctly.
4. **Verify remaining work** - Track URL open and alt-screen checks against `tasks.md` before closing the SPEC.
