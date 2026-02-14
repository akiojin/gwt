# タスクリスト: 起動時更新チェック堅牢化（遅延 + 再試行）

## Phase 1: セットアップ

- [ ] T001 [US1] SPEC-a3daf499 の仕様・計画を確定する `specs/SPEC-a3daf499/spec.md`

## Phase 2: 基盤

- [ ] T002 [US1] 起動時更新チェックヘルパーのI/Oを定義する `gwt-gui/src/lib/update/startupUpdate.ts`
- [ ] T003 [US1] 失敗ログの方針を統一する `crates/gwt-tauri/src/commands/update.rs`

## Phase 3: ストーリー 1

- [ ] T004 [US1] 起動時更新チェックの再試行仕様テストを追加する `gwt-gui/src/lib/update/startupUpdate.test.ts`
- [ ] T005 [US1] 起動時遅延 + 再試行ロジックを実装する `gwt-gui/src/lib/update/startupUpdate.ts`
- [ ] T006 [US1] App 起動時フローをヘルパー連携へ置換する `gwt-gui/src/App.svelte`
- [ ] T007 [US1] 起動時バックグラウンド更新失敗を warn ログ化する `crates/gwt-tauri/src/app.rs`

## Phase 4: ストーリー 2

- [ ] T008 [US2] 更新通知表示処理を共通化して重複表示を防ぐ `gwt-gui/src/App.svelte`
- [ ] T009 [US2] 手動更新チェックの既存挙動維持を確認する `gwt-gui/src/App.svelte`

## Phase 5: 仕上げ・横断

- [ ] T010 [P] [共通] フロントエンドテストを実行する `gwt-gui/src/lib/update/startupUpdate.test.ts`
- [ ] T011 [P] [共通] Rust テストを実行する `crates/gwt-tauri/src/commands/update.rs`
