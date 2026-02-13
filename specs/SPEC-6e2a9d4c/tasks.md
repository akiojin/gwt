# タスクリスト: Host OS 起動時の空タブ防止（Issue #1029）

## Phase 1: セットアップ

- [x] T001 [P] [共通] `spec.md` / `plan.md` / `tasks.md` を作成
- [x] T002 [P] [共通] `specs/specs.md` に仕様一覧を追加

## Phase 2: Windows Host 起動安定化

- [x] T003 [US2] `crates/gwt-core/src/terminal/pty.rs` に Windows `.cmd/.bat` ラップ起動ロジックを追加
- [x] T004 [US2] `crates/gwt-core/src/terminal/pty.rs` に起動コマンド解決ユニットテストを追加

## Phase 3: 空タブ防止

- [x] T005 [US1] `crates/gwt-core/src/terminal/pane.rs` に pane エラー状態設定メソッドを追加
- [x] T006 [US1] `crates/gwt-tauri/src/commands/terminal.rs` の `stream_pty_output` で read error を捕捉し、エラーメッセージ保存/通知を追加
- [x] T007 [US1] 失敗時でも `Press Enter to close this tab.` が表示されることを維持
- [x] T008 [US1] `crates/gwt-core/src/terminal/pane.rs` にエラー状態設定ユニットテストを追加

## Phase 4: 検証

- [x] T009 [US1] `cargo test -q -p gwt-core terminal::pty` を実行
- [x] T010 [US1] `cargo test -q -p gwt-core terminal::pane` を実行
- [x] T011 [US1] `cargo test -q -p gwt-tauri commands::terminal` を実行
