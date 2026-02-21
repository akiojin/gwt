# タスクリスト: PRタブへのWorkflow統合とブランチ状態表示

## 依存関係

- T002 → T003, T004 (Rust型拡張が先)
- T003, T004 → T005 (GraphQLクエリ拡張が先)
- T005 → T008 (update_pr_branch コマンドは型拡張後)
- T006 → T007, T009, T010, T011 (TypeScript型拡張が先)
- T009 → T012 (Checks セクションが先、Workflow タブ削除が後)

## Phase 1: セットアップ

- [x] T001 [P] [共通] 既存テストが全て通ることを確認する `cargo test` `cd gwt-gui && pnpm test`

## Phase 2: 基盤 - Rust 型拡張

- [x] T002 [US1] [テスト] `PrStatusInfo` に `merge_state_status` 、`WorkflowRunInfo` に `is_required` を追加するシリアライゼーションテストを追加する（RED） `crates/gwt-core/src/git/pullrequest.rs`
- [x] T003 [P] [US1] [実装] `PrStatusInfo` に `merge_state_status: Option<String>` フィールドを追加する `crates/gwt-core/src/git/pullrequest.rs`
- [x] T004 [P] [US2] [実装] `WorkflowRunInfo` に `is_required: Option<bool>` フィールドを追加する `crates/gwt-core/src/git/pullrequest.rs`

## Phase 3: ストーリー 1 & 3 - GraphQL クエリ拡張と mergeStateStatus

- [x] T005 [US1] [テスト] `build_pr_detail_query` に `mergeStateStatus` と `isRequired` が含まれることを検証するテストを追加する（RED） `crates/gwt-core/src/git/graphql.rs`
- [x] T006 [P] [US1] [実装] TypeScript 型 `PrStatusInfo` に `mergeStateStatus` 、`WorkflowRunInfo` に `isRequired` を追加する `gwt-gui/src/lib/types.ts`
- [x] T007 [US1] [テスト] PrStatusSection の Checks セクション表示テストを追加する（折りたたみ・展開・空状態） `gwt-gui/src/lib/components/PrStatusSection.test.ts`
- [x] T008 [US3] [テスト] `update_pr_branch` Tauri コマンドのシリアライゼーションテストを追加する `crates/gwt-tauri/src/commands/pullrequest.rs`

## Phase 4: ストーリー 1 - GraphQL 実装

- [x] T009 [US1] [実装] `build_pr_detail_query` に `mergeStateStatus` フィールドを追加し、パーサーで `merge_state_status` を抽出する `crates/gwt-core/src/git/graphql.rs`
- [x] T010 [US2] [実装] `build_pr_detail_query` の CheckRun に `isRequired(pullRequestNumber: $prNumber)` を追加し、パーサーで `is_required` を抽出する `crates/gwt-core/src/git/graphql.rs`
- [x] T011 [US3] [実装] `update_pr_branch` Tauri コマンドを追加する（`gh api -X PUT` で REST API 呼び出し） `crates/gwt-tauri/src/commands/pullrequest.rs`

## Phase 5: ストーリー 1 & 2 & 3 - フロントエンド実装

- [x] T012 [US1] [実装] PrStatusSection に折りたたみ式 Checks セクションを追加する（ワークフロー一覧・ステータスアイコン・クリックでCIログ） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T013 [US2] [実装] Checks セクション内の各 CheckRun に `isRequired` バッジを表示する `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T014 [US3] [実装] Merge メタ行を拡張し、mergeStateStatus 表示と Update Branch ボタンを追加する `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T015 [US3] [テスト] Merge 行の mergeStateStatus 表示と Update Branch ボタンの表示/非表示/クリックテスト `gwt-gui/src/lib/components/PrStatusSection.test.ts`

## Phase 6: ストーリー 4 - Workflow タブ削除と統合

- [x] T016 [US4] [テスト] WorktreeSummaryPanel の5タブ構成（Workflow タブ非存在）テストを追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [x] T017 [US4] [実装] WorktreeSummaryPanel から Workflow タブを削除し、SummaryTab 型を更新する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T018 [US4] [実装] PrStatusSection に `onOpenCiLog` コールバックを渡し、Workflow 操作を統合する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 7: 仕上げ・横断

- [x] T019 [P] [共通] 既存テストの Workflow タブ参照を全て更新・除去する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [x] T020 [P] [共通] 全テスト通過を確認する `cargo test` `cd gwt-gui && pnpm test`
- [x] T021 [共通] `cargo clippy --all-targets --all-features -- -D warnings` で lint 通過を確認する
- [x] T022 [共通] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` で型チェック通過を確認する
