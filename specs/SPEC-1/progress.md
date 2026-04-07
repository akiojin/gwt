# Progress: SPEC-1 - Terminal Emulation

## Progress
- Status: `done`
- Phase: `Done`
- Task progress: `59/59` checked in `tasks.md`
- Artifact refresh: `2026-04-07T01:57:01Z`

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
- Full-screen panes that keep `max_scrollback == 0` now maintain a pane-local in-memory snapshot history, render scrollbar chrome against that cache, and keep selection/copy aligned with the visible historical frame.
- Snapshot-backed scrollback stays frozen on the chosen historical frame while new output arrives and only returns to live-follow when the user scrolls back to the newest frame.
- Terminal-focus input is now normalized so leaked SGR wheel reports are converted back into mouse input instead of being echoed into the pane as literal `[<...M` text.
- Hover-only `Moved` floods are now dropped at the event layer, reducing redraw pressure during Terminal.app trackpad gestures.
- Consecutive wheel events are now drained as a bounded burst before redraw, so Terminal.app trackpad floods no longer force one full frame render per raw scroll event.
- Snapshot-backed scrollbar metrics now use the pane viewport height as the thumb baseline, so short frame histories render a legible scrollbar length instead of a single-cell indicator.
- PTY output chunks are now coalesced per session within each event-loop drain before `Message::PtyOutput` dispatch, so snapshot-backed scrollback tracks drawn frames instead of reader-chunk intermediate states.
- Full-screen cache history now records every distinct VT-interpreted frame (including overwrite / clear redraws) while deduplicating consecutive identical frames, so the visible frame always matches terminal semantics and prior distinct frames remain reviewable.
- Snapshot progression no longer depends on viewport-shift overlap heuristics; blank history prefixes are still pruned so topmost snapshot scrollback never produces a phantom blank screen.
- Alternate-screen panes now prefer snapshot-backed scrolling and scrollbar metrics even when main-screen row scrollback metadata is non-zero, so thumb movement always matches visible frame changes.
- Session viewport handling is now unified under `VtState`: rendering, scrollbar metrics, URL hit-testing, and selection copy all consume the same visible cache surface API.
- Snapshot history now prunes leading blank frames whenever newer non-blank frames exist, so topmost snapshot scrollback always lands on visible content.
- Snapshot live-to-history transition now applies exact one-step movement, fixing the off-by-one jump that skipped one frame on the first upward scroll.
- SGR leak normalization now uses inter-character inactivity timing, preventing delayed `[<...M` fragments from leaking into pane output while preserving mouse-wheel reconstruction.
- SGR leak normalization now runs regardless of terminal-focus state, so leaked wheel sequences can still recover into mouse scrolling when focus handoff has not happened yet.
- Snapshot capture now tolerates redraw churn by preserving any distinct visible frame, preventing history starvation on dynamic full-screen panes without overlap-score tuning.
- Acceptance and TDD checklists now reflect that the implementation tasks are complete and backed by focused verification evidence.

## Next
- Run the reviewer walkthrough in `quickstart.md` if manual confirmation is still required for release evidence.
