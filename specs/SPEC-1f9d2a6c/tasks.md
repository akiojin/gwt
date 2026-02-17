# タスクリスト: Window復元時の無限生成防止

## Phase 1: テスト設計（TDD観点）

- [x] T001 [P] [US1] バックエンド復元リーダーロックのテストケースを定義（main限定/拒否/期限切れ/解放）
- [x] T002 [P] [US2] フロント復元リーダーラッパのテストケースを定義（non-mainで未実行/command呼び出し/失敗フォールバック）

## Phase 2: バックエンド実装

- [x] T003 [US1] `AppState` に `window_session_restore_leader` と状態型を追加 `crates/gwt-tauri/src/state.rs`
- [x] T004 [US1] `try_acquire_window_session_restore_leader` と `release_window_session_restore_leader` を実装 `crates/gwt-tauri/src/state.rs`
- [x] T005 [US1] 復元リーダー command を追加 `crates/gwt-tauri/src/commands/window.rs`
- [x] T006 [US1] `generate_handler!` に command 登録 `crates/gwt-tauri/src/app.rs`

## Phase 3: フロント実装

- [x] T007 [US2] `windowSessionRestoreLeader.ts` を backend command ラッパへ置換 `gwt-gui/src/lib/windowSessionRestoreLeader.ts`
- [x] T008 [US2] `App.svelte` 復元処理を `await` command ベースへ切替 `gwt-gui/src/App.svelte`
- [x] T009 [US3] `resolveCurrentWindowLabel` の内部メタデータ fallback を削除 `gwt-gui/src/App.svelte`

## Phase 4: テスト実装と検証

- [x] T010 [US1] バックエンド復元リーダーロックのユニットテストを追加 `crates/gwt-tauri/src/state.rs`
- [x] T011 [US2] フロント復元リーダーラッパのユニットテストを更新 `gwt-gui/src/lib/windowSessionRestoreLeader.test.ts`
- [x] T012 [共通] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] T013 [共通] `cargo test -p gwt-tauri -- --nocapture`
- [x] T014 [共通] `cd gwt-gui && pnpm test src/lib/windowSessionRestoreLeader.test.ts`
- [x] T015 [共通] `cd gwt-gui && pnpm exec svelte-check --tsconfig ./tsconfig.json`
