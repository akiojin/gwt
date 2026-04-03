# Quickstart: SPEC-1 - Terminal Emulation

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and open a workspace with an active terminal tab.
2. Emit ANSI-heavy output and confirm colors plus cursor movement render correctly.
3. Emit `https://example.com/docs` into the active session, hold `Ctrl`, click the URL, and confirm the platform opener receives the full URL.
4. Emit a long URL that soft-wraps across two terminal rows and confirm the underline/click target remains intact on both rows.
5. Run `cargo test -p gwt-tui pty_output_renders_into_session_surface -- --nocapture` and record the passing output.
6. Run `cargo test -p gwt-tui ctrl_click_on_url_invokes_opener_with_full_url -- --nocapture` plus `cargo test -p gwt-tui wrapped_url -- --nocapture`, then record the passing output.

## Expected Result
- The reviewer sees live vt100-backed session rendering rather than placeholder text.
- `Ctrl+click` opens the visible URL from the active session pane.
- Wrapped URLs stay underlined and clickable across every visible row segment.
- No gap remains in `tasks.md` for URL opening or wrapped-URL handling.
