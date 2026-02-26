# タスクリスト: Worktree詳細ビューでMergeableクリックによるマージ実行

## Phase 1: 基盤

- [ ] T001 [US1] [toastBus テスト + 実装] gwt-gui/src/lib/toastBus.ts, gwt-gui/src/lib/toastBus.test.ts
- [ ] T002 [US1] [Backend merge コマンド テスト + 実装] crates/gwt-tauri/src/commands/pullrequest.rs
- [ ] T003 [US1] [App.svelte toastBus 購読] gwt-gui/src/App.svelte

## Phase 2: UIコンポーネント

- [ ] T004 [US2] [MergeConfirmModal テスト + 実装] gwt-gui/src/lib/components/MergeConfirmModal.svelte, gwt-gui/src/lib/components/MergeConfirmModal.test.ts
- [ ] T005 [US1,US3] [PrStatusSection バッジボタン化 テスト + 実装] gwt-gui/src/lib/components/PrStatusSection.svelte, gwt-gui/src/lib/components/PrStatusSection.test.ts

## Phase 3: 統合

- [ ] T006 [US1,US4] [WorktreeSummaryPanel マージフロー統合] gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte
- [ ] T007 [全体検証] 全テスト GREEN + 型チェック + Lint
