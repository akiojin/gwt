# Progress: SPEC-1776

## 2026-04-02: Normal-mode virtual terminal viewport

### Progress

- Replaced the explicit PTY copy mode with an always-on transcript-backed viewport for Agent and Shell tabs
- Enabled mouse capture in the Main layer so wheel / trackpad scroll and drag-selection work directly in normal mode
- Kept session-scoped raw PTY transcripts as the source of truth for history rendering, while preserving live follow at the bottom
- Added RED/GREEN coverage for keyboard scrollback, wheel scrollback, drag-copy, viewport freeze during new PTY output, and historical ANSI rendering
- Removed the `LIVE` / `SCROLLED` status label after it proved to be diagnostic noise, and made PTY-bound key input / paste immediately snap the viewport back to the live tail

### Done

- Agent/Shell tabs now support scrollback and drag-copy directly in normal mode
- Scrolling away from the live tail no longer snaps back when new PTY output arrives
- Returning to the bottom or pressing `End` restores live follow
- Typing or pasting while scrolled back immediately restores the live viewport before forwarding the input

### Next

- Manual E2E: run a chatty agent, scroll up with the trackpad, confirm the viewport stays fixed while output continues, then drag-copy text and return to live with `End`

## 2026-04-01: Workspace parent directory bare repo auto-detection

### Progress

- ワークスペース親ディレクトリ（非gitディレクトリ）から起動するとBranches画面が空になる問題を修正
- `main.rs` に `resolve_repo_root` ヘルパーを追加: `detect_repo_type` + `find_bare_repo_in_dir` で bare repo を自動検出
- `load_branches` で bare repo の場合 `Branch::list_remote_from_origin()` を使用するよう修正
- 4件のユニットテスト追加（normal repo, bare repo parent, no repo fallback, bare directly）

### Done

- `/Users/akiojin/Workbench/gwt` からの起動でBranches画面にブランチが表示される

### Next

- 手動 E2E: ワークスペース親ディレクトリから `cargo run -p gwt-tui` → Branches にブランチが表示されることを確認

## 2026-04-01: Agent launch worktree resolution fix

### Progress

- `spawn_agent_session` でブランチ選択時に `working_dir` が常に `model.repo_root` に固定されていたバグを修正
- `resolve_branch_working_dir` ヘルパーで `WorktreeManager` を使い、対象ブランチの worktree パスを builder・skill registration より前に解決
- `launch.rs` に auto_worktree フォールバックのユニットテストを追加

### Done

- Branches → ブランチ選択 → Launch Agent → 対象ブランチの worktree ディレクトリで agent が起動

### Next

- 手動 E2E: Branches → develop 選択 → Launch Agent → agent の cwd が develop worktree であることを確認

## 2026-04-01: Remote HEAD alias and deep PTY scrollback

### Progress

- Confirmed that `origin` in Branches was not a real branch but `refs/remotes/origin/HEAD` collapsing to `%(refname:short)` during remote ref listing
- Moved the fix to `gwt-core` remote branch enumeration so `origin` is filtered before the TUI builds Branch items
- Confirmed that Main PTY copy mode only used `vt100::Parser` in-memory history and never read the pane transcript persisted on disk
- Wired `Message::PtyOutput` to persist raw PTY bytes into pane scrollback files, then built a dedicated copy-mode history parser from that transcript so old output remains visible with ANSI styling intact

### Done

- Branches no longer shows the fake `origin` row from remote HEAD aliases
- Main PTY copy mode can now scroll into output older than the live parser buffer while preserving ANSI colors

### Next

- Manual E2E: open a long-running agent pane, enter copy mode, jump to the oldest visible output, and confirm historical colored output still renders correctly

## 2026-04-01: Branch view-mode filter fix

### Progress

- Reproduced that `Local / Remote / All` did not react to remote-tracking refs named like `origin/main`
- Added RED coverage around `BranchItem::from_branch()` so remote refs and local refs are distinguished from the gwt-core branch shape, not from ad-hoc test fixtures
- Fixed remote detection and protected-branch normalization so `origin/*` and `remotes/origin/*` are both classified correctly in the Branches tab

### Done

- Branches view-mode filtering now works for actual remote branch names returned by git

### Next

- Manual E2E: open Branches and cycle `All / Local / Remote` on a repo with remote-tracking refs to confirm counts and rows update as expected

## 2026-04-01: Management panel detail polish

### Progress

- Reordered the management tabs to `Branches / SPECs / Issues / Versions / Settings / Logs` so keyboard traversal matches the intended information architecture
- Fixed SPEC detail resolution by keeping the real `SPEC-*` directory name separate from the display ID in `metadata.json`
- Replaced plain-text detail rendering in SPECs / Issues / Versions with a shared Markdown renderer for headings, lists, blockquotes, and code fences
- Reworked Versions into a lightweight version-history view with the latest 10 semantic tags, range labels, commit counts, changelog-derived preview text, and Markdown detail
- Switched Logs loading to workspace JSONL files under `~/.gwt/logs/{workspace}/` and surfaced structured fields plus new UI flow logs for tab switches, detail opens, refreshes, and version summary failures

### Done

- SPEC / Issue details now open reliably and render as readable Markdown
- Versions now show useful summaries without requiring raw `git show` output
- Logs now contain and display enough structured context to debug management-panel flows

### Next

- Run full repo verification (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`) and confirm manual TUI behavior

## 2026-04-01: Constitution path unified under `.gwt`

### Progress

- Found a root-cause mismatch: runtime logic already treated `.gwt/memory/constitution.md` as canonical, but `gwt-core` still embedded `memory/constitution.md` at compile time
- Switched the managed asset source to `.gwt/memory/constitution.md` and stopped counting the legacy root path as satisfying registration status
- Removed the tracked duplicate `memory/constitution.md` file and updated stale path references in source comments and SPEC docs

### Done

- Skill registration now has a single canonical constitution source: `.gwt/memory/constitution.md`

### Next

- Verify on a clean checkout that `cargo test -p gwt-core -p gwt-tui` passes without recreating `memory/constitution.md`

## 2026-04-01: PTY paste input

### Progress

- Confirmed that `Enter` and pasted text were on different paths: `Enter` was normalized, but terminal `Paste` events were ignored
- Enabled bracketed paste at the terminal boundary and routed `Event::Paste(String)` through the Elm update loop
- Added a PTY integration test proving that multi-line pasted text is forwarded to the active pane as one payload

### Done

- Text paste into Agent/Shell tabs now reaches the PTY reliably, including embedded newlines

### Next

- Manual E2E: paste multi-line text into an Agent/Shell tab in Terminal.app and confirm the full payload arrives without splitting into per-key behavior

## 2026-04-01: Main PTY copy mode (superseded by 2026-04-02 normal-mode viewport)

### Progress

- Added a dedicated PTY copy mode on `Ctrl+G,m` for the active Agent/Shell tab
- Kept terminal-native selection/copy in normal mode by enabling mouse capture only in management screens or copy mode
- Added keyboard scrollback navigation, mouse wheel scrolling, drag-to-copy, and viewport freeze while PTY output continues
- Removed stale `Ctrl+G,n` launch shortcut references so agent launch stays anchored on Branches `Enter`

### Done

- Main PTY now supports explicit copy/scroll behavior without regressing terminal-native copy outside copy mode

### Next

- Manual E2E: enter copy mode in an Agent/Shell tab, scroll with trackpad, drag-copy text, then exit and verify the viewport snaps back to live output

## 2026-04-01: Logs trackpad scroll fix

### Progress

- Routed `MouseInput` scroll events to the Logs screen navigation handler
- Added RED/GREEN coverage for `ScrollUp` and `ScrollDown` on the Logs tab
- Cleaned package-local clippy failures encountered during verification

### Done

- Trackpad / mouse wheel scrolling now moves the Logs selection so older entries are reachable

### Next

- Manual E2E: open Logs tab and confirm trackpad scrolling moves through historical entries
## 2026-04-01: Session auto-close on exit (superseded)

### Progress

- Added PTY termination polling in `Model::apply_background_updates()`
- Tried automatically closing Agent and Shell tabs when their underlying process exited
- This behavior was later reverted the same day because it hid the final transcript/error output before the user could inspect it

### Done

- Superseded by the follow-up change below: exited Agent and Shell sessions now stay visible with completed/error state until manual close

### Next

- No further action for this approach; preserved only as historical context

## 2026-04-01: Preserve exited session tabs for error inspection

### Progress

- Reproduced the user-facing failure mode: a short-lived agent/shell process printed its final error and then `Model::apply_background_updates()` immediately removed the session tab, sending the UI back to Branches before the message could be read
- Replaced the auto-close path with session-status updates so `PaneStatus::Completed` / `PaneStatus::Error` are reflected into `SessionTab.status` while the tab and transcript remain visible
- Added RED/GREEN coverage for completed agent and shell sessions plus focus retention on the exited tab

### Done

- Exited Agent and Shell sessions now remain open with completed/error state until the user closes them explicitly

### Next

- Manual E2E: launch an agent or shell that exits immediately, confirm the final output stays visible in the tab, then close it with `Ctrl+G,x`

## 2026-04-02: Separate local SPEC viewer from GitHub Issues

### Progress

- Replaced the old `Issues` tab behavior that mixed local SPEC rows into GitHub Issue results
- Switched `SPECs` detail from `spec.md`-only rendering to a sectioned artifact viewer backed by the local SPEC loader
- Fixed `gwt-core` local SPEC aggregation so contracts/checklists are visible to the detail viewer instead of being dropped from `LocalSpecDetail.sections`
- Updated the canonical docs and skill guidance so local SPEC artifacts remain the source of truth and GitHub Issues stay an optional related record

### Done

- `Issues` now loads GitHub Issue cache / refresh results only
- `SPECs` detail now exposes `spec / plan / tasks / research / data-model / quickstart / checklists / contracts`
- Local SPEC detail aggregation includes named contract/checklist artifacts

### Next

- Manual E2E: open `SPECs` detail and verify section switching, then refresh `Issues` against a repo with cached/open GitHub issues

## 2026-03-27: Phase 0 + Phase 1 Core Implementation

### Progress

- Created `crates/gwt-tui/` crate with ratatui 0.29, crossterm 0.28, vt100 0.15
- Implemented all Phase 0 tasks (T001-T004): scaffold, Cargo.toml, main.rs, build verification
- Implemented Phase 1 renderer (T010-T012): VT100 Screen → ratatui Buffer with color/attribute mapping
- Implemented Phase 1 state (T020-T021): TuiState with tab management, bounds-safe navigation
- Implemented Phase 1 event (T022-T023): EventLoop multiplexing crossterm + PTY channel + tick timer
- Implemented Phase 1 keybind (T030-T031): Ctrl+G prefix state machine with 2s timeout, full action set
- Implemented Phase 1 UI components (T040-T043): tab_bar, terminal_view, status_bar with snapshot tests
- Implemented Phase 1 app (T050): App struct with event dispatch and render cycle
- 53 tests passing, clippy clean, gwt-core tests unaffected

### Done

- Phase 0: Complete (T001-T004)
- Phase 1 Renderer: Complete (T010-T012)
- Phase 1 State + Event: Complete (T020-T023)
- Phase 1 KeyBind: Complete (T030-T031)
- Phase 1 UI Components: Complete (T040-T043)
- Phase 1 App skeleton: Complete (T050)
- Phase 1 Verification partial: T061 complete (all tests pass)

### Next

- T051: Wire shell tab creation (Ctrl+G,c) via PaneManager::spawn_shell()
- T052: Wire PTY I/O (key → write_input, PTY reader → process_bytes → render)
- T053: Wire terminal resize → PaneManager::resize_all()
- T054: Implement scrollback scroll mode
- T060: Integration test with live PTY
