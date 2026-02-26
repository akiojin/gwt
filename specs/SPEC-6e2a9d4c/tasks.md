# タスクリスト: Host OS 起動時の空タブ防止（Issue #1029）

## Phase 1: セットアップ

- [x] T001 [P] [共通] `spec.md` / `plan.md` / `tasks.md` を作成
- [x] T002 [P] [共通] `specs/specs.md` に仕様一覧を追加

## Phase 2: Windows Host 起動安定化

- [x] T003 [US2] `crates/gwt-core/src/terminal/pty.rs` に Windows PowerShell ラップ起動ロジック（`pwsh` 優先 / `powershell.exe` フォールバック）を追加
- [x] T004 [US2] `crates/gwt-core/src/terminal/pty.rs` に起動コマンド解決ユニットテストを追加

## Phase 3: 空タブ防止

- [x] T005 [US1] `crates/gwt-core/src/terminal/pane.rs` に pane エラー状態設定メソッドを追加
- [x] T006 [US1] `crates/gwt-tauri/src/commands/terminal.rs` の `stream_pty_output` で read error を捕捉し、エラーメッセージ保存/通知を追加
- [x] T007 [US1] 失敗時でも `Press Enter to close this tab.` が表示されることを維持
- [x] T008 [US1] `crates/gwt-core/src/terminal/pane.rs` にエラー状態設定ユニットテストを追加

## Phase 4: 検証（既存）

- [x] T009 [US1] `cargo test -q -p gwt-core terminal::pty` を実行
- [x] T010 [US1] `cargo test -q -p gwt-core terminal::pane` を実行
- [x] T011 [US1] `cargo test -q -p gwt-tauri commands::terminal` を実行

## Phase 5: Backend-gated event emission（根本修正）

- [ ] T012 [US3] `pane.rs` に `frontend_ready` フィールド + `is_frontend_ready` / `set_frontend_ready` メソッド追加
- [ ] T013 [US3] `pane.rs` に `read_scrollback_tail_raw` メソッド追加
- [ ] T014 [US3] `stream_pty_output` メインreadループに ready gate 追加（FR-006）
- [ ] T015 [US3] `wsl_prompt_detect_and_inject` reader スレッドに ready gate 追加（FR-008）
- [ ] T016 [US3] `terminal_ready` Tauri コマンド追加（FR-007）
- [ ] T017 [US3] `app.rs` の invoke_handler に `terminal_ready` を登録
- [ ] T018 [US3] `TerminalView.svelte` の onMount 簡素化（FR-009）
- [ ] T019 [US3] `TerminalView.test.ts` テスト更新

## Phase 6: 検証（根本修正）

- [ ] T020 [US3] `cargo test -p gwt-core` で pane.rs テスト通過
- [ ] T021 [US3] `cd gwt-gui && pnpm test src/lib/terminal/TerminalView.test.ts` 通過
- [ ] T022 [US3] `cargo clippy --all-targets --all-features -- -D warnings` 通過
- [ ] T023 [US3] `cargo fmt` 通過
- [ ] T024 [US3] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` 通過
