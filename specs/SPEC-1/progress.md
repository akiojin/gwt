# Progress: SPEC-1 - Terminal Emulation

## Progress
- Status: `done`
- Phase: `Done`
- Task progress: `28/28` checked in `tasks.md`
- Artifact refresh: `2026-04-06T15:03:56Z`

## Done
- Supporting artifacts now exist for planning, execution tracking, and review.
- URL detection and alt-screen verification tests exist at the renderer layer.
- `PtyOutput` now feeds a per-session vt100 surface and the session pane renders live terminal content instead of a placeholder.
- `Ctrl+click` URL open now resolves visible URL regions from the active session pane and invokes the platform opener with the full URL.
- Wrapped URLs are now detected across soft-wrapped terminal rows and remain underlined/clickable across every visible segment.
- Terminal sessions now keep viewport-local scrollback state, expose overflow-only scrollbar chrome, and restore the cursor against the text area even when the gutter is present.
- Mouse-wheel scrolling now freezes live follow against vt100 scrollback, and drag selection copies from the visible scrollback viewport through `contents_between()`.
- Session-pane mouse interactions now re-focus the terminal before scrollback routing, so wheel scrolling works from the default management-focus state instead of dropping the first event.
- Terminal startup now disables alternate-scroll mode so Terminal.app trackpad gestures are not rewritten into cursor keys while gwt owns the alternate screen.
- Terminal.app-specific `Drag(Right)` gesture sequences now fall back to scrollback motion, matching the observed crossterm event stream when wheel events are absent.
- Acceptance and TDD checklists now reflect that the implementation tasks are complete and backed by focused verification evidence.

## Next
- Run the reviewer walkthrough in `quickstart.md` if manual confirmation is still required for release evidence.
