# 実装計画: PRタブへのWorkflow統合とブランチ状態表示

**仕様ID**: `SPEC-de3290fc` | **日付**: 2026-02-22 | **仕様書**: `specs/SPEC-de3290fc/spec.md`

## 目的

- Workflow タブを PR タブに統合し、タブ構成を5タブ（Summary / Git / Issue / PR / Docker）に簡素化する
- PR の Checks に Required/Non-required の識別バッジを付与する
- ブランチの behind/conflict 状態を可視化し、Update Branch アクションを提供する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/`, `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **外部連携**: GitHub GraphQL API v4 (gh CLI 経由)、GitHub REST API (`PUT /repos/{owner}/{repo}/pulls/{number}/update-branch`)
- **テスト**: cargo test (Rust) / vitest (Frontend)
- **前提**: gh CLI が認証済み

## 実装方針

### Phase 1: バックエンド - GraphQL クエリ拡張と型追加

GraphQL クエリに2つのフィールドを追加し、対応する Rust 型を拡張する。

1. **`mergeStateStatus` フィールド追加** (`crates/gwt-core/src/git/graphql.rs`)
   - `build_pr_status_query` と `build_pr_detail_query` の PullRequest フィールドに `mergeStateStatus` を追加
   - パース関数で `merge_state_status` を抽出

2. **`isRequired` フィールド追加** (`crates/gwt-core/src/git/graphql.rs`)
   - CheckRun の inline fragment に `isRequired(pullRequestNumber: $prNumber)` を追加
   - 注意: `isRequired` は PR 番号を引数に取るため、detail クエリでのみ使用する（一括クエリでは PR 番号が動的なので省略）

3. **Rust 型の拡張** (`crates/gwt-core/src/git/pullrequest.rs`)
   - `PrStatusInfo` に `merge_state_status: Option<String>` を追加
   - `WorkflowRunInfo` に `is_required: Option<bool>` を追加

4. **Update Branch コマンド追加** (`crates/gwt-tauri/src/commands/pullrequest.rs`)
   - `update_pr_branch` Tauri コマンドを追加
   - `gh api -X PUT /repos/{owner}/{repo}/pulls/{number}/update-branch` を実行

### Phase 2: フロントエンド - 型定義と PrStatusSection 拡張

1. **TypeScript 型の拡張** (`gwt-gui/src/lib/types.ts`)
   - `PrStatusInfo` に `mergeStateStatus` を追加
   - `WorkflowRunInfo` に `isRequired` を追加

2. **PrStatusSection コンポーネント拡張** (`gwt-gui/src/lib/components/PrStatusSection.svelte`)
   - Merge メタ行の拡張: `mergeStateStatus` に基づく表示（Behind/Blocked/Clean/Dirty/Draft/Unstable）
   - Update Branch ボタン: `mergeStateStatus === "BEHIND"` の場合に表示、クリックで Tauri コマンド実行
   - Checks セクション追加: 折りたたみ式アコーディオン、ワークフロー一覧表示
   - Required バッジ: `isRequired === true` の CheckRun に「required」ピルバッジ表示
   - `onUpdateBranch` コールバック prop を追加

3. **ヘルパー関数の移動** (`gwt-gui/src/lib/prStatusHelpers.ts`)
   - `workflowStatusIcon`, `workflowStatusClass`, `workflowStatusText` を PrStatusSection 内で使用

### Phase 3: WorktreeSummaryPanel 整理

1. **Workflow タブ削除** (`gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`)
   - `SummaryTab` 型から `"workflow"` を削除
   - タブボタンから Workflow を削除
   - `activeTab === "workflow"` の条件分岐を削除
   - `activeTab === "pr" || activeTab === "workflow"` のガード条件を `activeTab === "pr"` に変更

2. **PR タブへの統合**
   - PrStatusSection に Workflow 関連のデータと操作を渡す
   - `openWorkflowRun` 関数を PrStatusSection 内で使用するためのコールバック化

## テスト

### バックエンド

- `mergeStateStatus` パースのユニットテスト（各 enum 値の正常パース）
- `isRequired` パースのユニットテスト（true/false/null のケース）
- `update_pr_branch` コマンドのシリアライゼーションテスト
- 既存 GraphQL テストの更新（新フィールド追加に伴う期待値更新）

### フロントエンド

- PrStatusSection: Checks セクションの表示/折りたたみテスト
- PrStatusSection: Required バッジの表示テスト
- PrStatusSection: Merge 行の mergeStateStatus 表示テスト
- PrStatusSection: Update Branch ボタンの表示/クリックテスト
- WorktreeSummaryPanel: 5タブ構成（Workflow タブ非存在）テスト
- 既存テストの更新（Workflow タブ参照の除去）
