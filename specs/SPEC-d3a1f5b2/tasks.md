# タスクリスト: Cmd+Q 二重確認によるアプリ終了

## Phase 1: 原因調査

- [ ] T001 [P] [US1] 現行ビルドで Cmd+Q 時にプロセスが実際に死亡するか検証し、api.prevent_exit() の動作を確認する `crates/gwt-tauri/src/app.rs`

## Phase 2: バックエンド - 状態管理

- [x] T002 [US1] AppState に `quit_confirm_requested_at: Mutex<Option<Instant>>` を追加する `crates/gwt-tauri/src/state.rs`
- [x] T003 [US1] quit_confirm 状態管理のユニットテストを書く（TDD: RED） `crates/gwt-tauri/src/state.rs`
- [x] T004 [US1] quit_confirm 状態管理のメソッドを実装する（TDD: GREEN） `crates/gwt-tauri/src/state.rs`

## Phase 3: バックエンド - ExitRequested ハンドラ

- [x] T005 [US1][US3][US4][US5] ExitRequested ハンドラの二重確認ロジックテストを書く（TDD: RED） `crates/gwt-tauri/src/app.rs`
- [x] T006 [US1][US3][US4][US5] ExitRequested ハンドラを二重確認ロジックに書き換える `crates/gwt-tauri/src/app.rs`
- [x] T007 [US2] `cancel_quit_confirm` Tauri コマンドを追加する `crates/gwt-tauri/src/commands/`
- [x] T008 [US4] エージェント警告ダイアログのロジックをExitRequestedから削除する `crates/gwt-tauri/src/app.rs`

## Phase 4: フロントエンド - トーストコンポーネント

- [x] T009 [US1] QuitConfirmToast コンポーネントのテストを書く（TDD: RED） `gwt-gui/src/lib/components/QuitConfirmToast.test.ts`
- [x] T010 [US1] QuitConfirmToast.svelte コンポーネントを実装する `gwt-gui/src/lib/components/QuitConfirmToast.svelte`
- [x] T011 [US2] トースト表示中の他操作検知（mousedown/keydown）によるリセットを実装する `gwt-gui/src/lib/components/QuitConfirmToast.svelte`
- [x] T012 [US1] App.svelte に QuitConfirmToast を組み込む `gwt-gui/src/App.svelte`

## Phase 5: クリーンアップ

- [ ] T013 [US4] exit_confirm_inflight フィールドと関連関数（try_begin_exit_confirm / end_exit_confirm）を削除する `crates/gwt-tauri/src/state.rs` `crates/gwt-tauri/src/app.rs`
- [x] T014 [P] cargo clippy / cargo test / svelte-check / vitest 全パスを確認する
