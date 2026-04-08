# Terminal Emulation -- Tasks

## Phase 1: URL Detection and Opening

- [x] T001 [P] Write RED tests for URL regex matching: single URL, multiple URLs per line, URLs with query params/fragments/parentheses, non-URL text.
- [x] T002 [P] Write RED tests for URL region tracking: verify screen coordinates map to correct URL strings.
- [x] T003 Implement URL regex matching utility that returns match ranges for a given string.
- [x] T004 Integrate URL detection into renderer: scan visible lines, apply underline style to URL regions.
- [x] T005 Implement URL region coordinate tracking for click detection.
- [x] T006 Write RED test for Ctrl+click on URL: verify platform opener is invoked with correct URL.
- [x] T007 Implement Ctrl+click handler: detect click within URL region, invoke `open`/`xdg-open` with URL.
- [x] T008 Write RED test for wrapped URLs spanning two terminal lines.
- [x] T009 Implement wrapped URL detection across adjacent lines.
- [x] T010 Verify all URL detection tests pass GREEN.

## Phase 2: Alt-Screen Buffer Verification

- [x] T011 [P] Write RED test: send DECSET 1049, write alt-screen content, send DECRST 1049, verify main scrollback preserved.
- [x] T012 [P] Write RED test: verify cursor position restores correctly after alt-screen exit.
- [x] T013 Run alt-screen tests against vt100 crate and verify GREEN (or document gaps).
- [x] T014 If gaps found, add workaround or document limitation in spec. *(no gaps found; vt100 handles alt-screen correctly)*

## Phase 3: Regression and Polish

- [x] T015 Run full existing test suite (17+ keybind tests, viewport tests) and verify no regressions.
- [x] T016 Run `cargo clippy` and `cargo fmt` on all changed files.
- [x] T017 Update SPEC-1 progress artifacts with verification results.

## Phase 4: Scrollback Interaction, Selection, And Scrollbar

- [x] T018 [P] Write RED test: mouse-wheel scroll moves the terminal viewport into scrollback and keeps live-follow disabled until the viewport returns to the bottom.
- [x] T019 [P] Write RED test: overflowing terminal history renders a right-edge scrollbar, while non-overflowing history does not reserve scrollbar chrome.
- [x] T020 [P] Write RED test: drag selection across visible scrollback copies the expected single-line and multi-line payload via `contents_between()`.
- [x] T021 Extend terminal session state with viewport and selection tracking in `model.rs`.
- [x] T022 Implement mouse-wheel scrollback control and live-follow restoration in `app.rs`.
- [x] T023 Implement drag-selection extraction/highlighting and clipboard copy in `app.rs` / `renderer.rs`.
- [x] T024 Render the overflow-only terminal scrollbar in the session surface and keep cursor placement correct when the gutter is present.
- [x] T025 Refresh SPEC-1 artifacts and rerun focused terminal interaction verification.
- [x] T026 Fix regression: session-pane mouse wheel now re-focuses the terminal before applying scrollback.
- [x] T027 Fix Terminal.app trackpad regression by disabling alternate-scroll mode during terminal startup and verify with focused startup + scroll/copy regression tests.
- [x] T028 Fix Terminal.app right-drag trackpad fallback by mapping vertical `Drag(Right)` deltas into scrollback motion and verify focused regression coverage.
- [x] T029 [P] Write RED tests: a full-screen pane with `max_scrollback == 0` still reveals previous frames, renders a scrollbar, and copies from the visible snapshot surface.
- [x] T030 [P] Write RED test: snapshot-backed scrollback stays on the selected historical frame while new output arrives and only returns to live at the newest frame.
- [x] T031 Implement pane-local in-memory screen snapshot history in `model.rs` with a fixed ring buffer and no transcript preload.
- [x] T032 Route render / selection / URL hit testing / scrollbar logic through the currently visible live-or-snapshot surface and verify focused regressions.
- [x] T033 [P] Write RED tests: ignore `MouseEventKind::Moved` floods and normalize leaked SGR wheel reports back into terminal mouse input without forwarding raw `[<...M` text to the PTY.
- [x] T034 Implement terminal-focus input normalization in `event.rs` / `main.rs` so leaked SGR mouse reports are swallowed or converted, and hover-only move events are dropped before redraw.
- [x] T035 [P] Write RED tests: consecutive wheel messages are drained as one bounded burst and the first following non-scroll message is preserved for the next event-loop iteration.
- [x] T036 Implement bounded wheel-burst batching in `main.rs` so Terminal.app trackpad floods do not force one redraw per raw scroll event.
- [x] T037 [P] Write RED test: snapshot-backed scrollbar metrics keep the thumb length proportional to the visible viewport height instead of collapsing to a single-cell frame indicator.
- [x] T038 Adjust snapshot scrollbar metrics in `app.rs` so thumb length tracks the pane viewport while row scrollback behavior stays unchanged.
- [x] T039 [P] Write RED test: PTY output chunks drained in one event-loop pass are coalesced per session so snapshot-backed scrollback does not expose intermediate chunk states.
- [x] T040 Coalesce per-session PTY output in `main.rs` before dispatching `Message::PtyOutput`, keeping snapshot capture aligned with rendered frame boundaries.
- [x] T041 [P] Write RED tests: full-screen snapshot cache keeps distinct redraw frames while deduplicating consecutive identical frames.
- [x] T042 Adjust the full-screen cache model in `model.rs` / `app.rs` so any distinct VT-interpreted frame is appended to history without relying on viewport-shift-only progression.
- [x] T043 [P] Write RED tests: blank-to-bottom-aligned frame transitions keep snapshot history usable while topmost scrollback never lands on a phantom empty frame.
- [x] T044 Remove overlap-based viewport-shift heuristics and keep snapshot history stable via distinct-frame capture plus blank-prefix pruning.
- [x] T045 [P] Write RED test: leading blank snapshots are pruned once newer non-blank frames exist so topmost scrollback never lands on an empty phantom frame.
- [x] T046 Prune leading blank snapshot prefixes in `model.rs` and preserve snapshot cursor clamping while keeping non-blank history intact.
- [x] T047 [P] Write RED test: first upward snapshot scroll from live-follow lands on the immediately previous snapshot and does not skip one frame.
- [x] T048 Fix snapshot scroll-up cursor calculation in `model.rs` so live-to-history transition applies exact one-step deltas.
- [x] T049 [P] Write RED test: leaked SGR wheel reports with short inter-character delays are still normalized as mouse input.
- [x] T050 Switch SGR leak timeout handling in `event.rs` to inter-character inactivity semantics so delayed fragments do not leak literal `[<...M` text.
- [x] T051 [P] Write RED test: leaked SGR wheel reports are still normalized when terminal pane is not focused.
- [x] T052 Apply SGR leak normalization regardless of terminal focus state and keep timeout tuned for practical trackpad sequence jitter.
- [x] T053 [P] Write RED tests: overlap-row churn and in-place redraws still remain reviewable as long as the resulting VT frame is distinct.
- [x] T054 Finalize snapshot capture in `model.rs`/`app.rs` around distinct-frame history semantics and update focused regression expectations.
- [x] T055 [P] Write RED tests: alternate-screen snapshot scrollback remains functional even when main-screen row scrollback already exists.
- [x] T056 Prefer snapshot-backed scroll path and scrollbar metrics while alternate-screen is active, regardless of non-zero row scrollback.
- [x] T057 Consolidate session viewport behavior behind `VtState` (`visible_screen_parser`, `scroll_viewport_lines`, `scrollbar_metrics`) so app-level mode branching is removed.
- [x] T058 Route render, URL hit-testing, and selection copy in `app.rs` through the same `VtState` visible-surface API.
- [x] T059 [P] Verify unified viewport path with focused + full `gwt-tui` tests, ensuring scrollbar movement and rendered surface stay synchronized.
- [x] T060 [P] Write RED tests: active Claude/Codex panes can hydrate scrollback from session `jsonl` history and navigate older transcript lines through the same viewport APIs.
- [x] T061 Implement transcript-backed scrollback mode in `model.rs` + `app.rs`, including source resolution (`~/.claude/projects`, `~/.codex/sessions`) and in-memory cache synchronization for active agent panes.
- [x] T062 Re-run focused scrollback regression suite (`cargo test -p gwt-tui scrollback -- --nocapture`) to verify transcript-backed history does not regress snapshot/row behavior.
- [x] T063 [P] Write RED tests: recent agent-pane row/snapshot cache keeps VT color attributes while transcript history remains available as an older fallback.
- [x] T064 Adjust `VtState` transcript fallback ordering so recent local cache is exhausted before plain-text transcript mode becomes active, and downward scroll returns to the oldest local cache before live-follow.
- [x] T065 Re-run focused transcript + scrollback regressions and verify scrollbar metrics / viewport routing stay coherent with the new cache-first ordering.
- [x] T066 [P] Write RED tests: Codex `function_call_output` and Claude `tool_result` transcript hydration preserve ANSI-styled tool output lines inside agent scrollback.
- [x] T067 Preserve raw tool-output transcript lines during Claude/Codex `jsonl` hydration in `app.rs` so transcript-backed scrollback does not drop colorized session output.
- [x] T068 [P] Write RED tests: snapshot-backed local cache + transcript overlap does not create a scrollbar dead zone before older unique history appears.
- [x] T069 Collapse overlapping snapshot/transcript tail inside `VtState` so viewport transitions and scrollbar metrics skip duplicated recent history.
- [x] T070 [P] Write RED tests: agent scrollback ignores alternate-screen launch/blank frame history and transcript selection prefers the session started nearest the gwt launch time.
- [x] T071 Replace agent-pane snapshot frame scrollback with normalized row scrollback, and select Claude/Codex transcript files by session metadata instead of worktree-global recency.
- [x] T072 [P] Write RED tests: agent panes ignore session-log hydration at runtime, preserve ANSI styles in in-memory history, and keep a larger row-scrollback budget than standard terminal panes.
- [x] T073 Remove transcript-backed runtime scrollback wiring from `app.rs` so PTY output is the only source of agent-pane history while the pane is alive.
- [x] T074 Simplify `VtState` agent scrollback to `AgentMemoryBacked` memory-only history that prefers normalized row scrollback, falls back to in-memory snapshots when rows never advance, and never uses transcript fallback.
- [x] T075 Refresh SPEC-1 artifacts, lessons learned, and focused/full verification for the memory-only agent scrollback design.
- [x] T076 Write RED tests and implement `forward_key_to_active_session()` live-follow reset so PTY-bound key input exits row/snapshot history before forwarding bytes.
- [x] T077 Write RED tests and implement agent-pane PTY mouse-scroll forwarding so SGR mouse-enabled agents receive wheel / right-drag scroll input directly instead of gwt-local scrollback.
- [x] T078 Keep agent scroll ownership capability-driven: PTY-mouse-enabled panes use PTY scroll, panes without that capability stay on local scrollback, and stale local scrollbar overlays are suppressed only in the PTY-owned path.
- [x] T079 Preserve intermediate agent full-screen redraw frames inside coalesced PTY payloads so local Codex snapshot scrollback remains deep enough to review earlier frames.
- [x] T080 [P] Write RED tests: agents without SGR mouse reporting keep wheel / right-drag scroll on local viewport history and never synthesize cursor-key PTY input.
- [x] T081 Implement capability-driven agent scroll routing in `app.rs` so only SGR mouse-enabled panes use PTY-owned scrolling, while local-scroll panes keep their scrollbar overlay.
- [x] T082 Refresh SPEC-1 artifacts, lessons learned, and focused/full verification for the corrected non-PTY agent scroll ownership model.
- [x] T083 [P] Write RED tests: Codex-style full-screen redraw panes promote vertical redraw shifts into local row history while preserving snapshot fallback when rows still cannot be derived.
- [x] T084 Implement redraw-shift row-cache normalization in `model.rs` and keep scroll-route diagnostics for local vs PTY-mouse ownership, including home-only repaint redraws and fixed-header status churn that do not emit `\x1b[2J\x1b[H`.
- [x] T085 [P] Write RED tests: terminal, snapshot, and agent panes render without any scrollbar gutter even when local history overflows.
- [x] T086 Remove the terminal scrollbar overlay and keep session text width equal to pane width for all local and PTY-owned scroll paths.
- [x] T087 [P] Write RED test: while an agent pane is browsing snapshot history, incoming PTY redraws that create row history do not replace the currently selected snapshot.
- [x] T088 Keep snapshot-backed history locked to the selected frame until live-follow resumes, even if new PTY output promotes fresh row history in the background.
- [x] T089 [P] Write RED test: Codex-style sparse same-offset redraw matches still derive local row history instead of falling back to page-sized snapshot scrolling.
- [x] T090 Extend redraw-shift detection to accept sparse same-offset matches when contiguous overlap is interrupted by progress/spinner churn.
- [x] T091 [P] Write RED tests: Codex launch configs include `--no-alt-screen` both in the shared launch builder and the wizard-built launch path.
- [x] T092 Prefer Codex inline mode by adding `--no-alt-screen` to Codex launch args so PTY output preserves scrollback without relying on fullscreen redraw reconstruction.
- [x] T093 [P] Write RED tests: invalid `Esc`-prefixed non-SGR sequences replay in original order, and pending normalized keys still traverse the shared keybind dispatch path.
- [x] T094 [P] Write RED tests: Terminal.app right-drag anchor state clears on outside mouse-up and session/focus changes, while transcript-ignore tests exercise real transcript discovery/parsing first.
- [x] T095 Implement ordered SGR-normalization fallback replay, share post-normalization dispatch for pending/polled messages, and make the transcript-ignore helper exercise real discovery/parsing without hydrating runtime history.
- [x] T096 Implement lazy scroll-debug logging and clear Terminal.app right-drag anchor state whenever terminal ownership changes or the drag ends outside the session pane.
- [x] T097 [P] Write RED test: once `AgentMemoryBacked` redraws are normalized into row history, snapshot storage collapses to a single live comparison baseline instead of growing hidden snapshot history.
- [x] T098 Rework `VtState` snapshot capture so agent panes reuse the current frame snapshot and keep only the latest live baseline whenever row-history scrollback is active.
- [x] T099 [P] Add a regression test covering visible row extraction semantics before replacing per-row selection reads with a single-pass scan.
- [x] T100 Replace `screen_visible_lines()` with a single-pass `vt100::Screen::rows()` walk so redraw-shift detection no longer pays the hidden O(rows²) `contents_between(...).nth(row)` cost on every frame.
