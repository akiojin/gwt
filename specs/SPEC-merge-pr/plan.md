# 実装計画: Worktree詳細ビューでMergeableクリックによるマージ実行

**仕様ID**: `SPEC-merge-pr` | **日付**: 2026-02-26 | **仕様書**: `specs/SPEC-merge-pr/spec.md`

## 目的

- Worktree詳細ビューのPRタブでMergeableバッジをクリックしてPRマージを実行可能にする

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2 (`crates/gwt-tauri/`)
- **フロントエンド**: Svelte 5 + TypeScript (`gwt-gui/`)
- **外部連携**: GitHub REST API via `gh api`
- **テスト**: cargo test / vitest
- **前提**: `gh` CLI がインストール済みかつ認証済み

## 実装方針

### Phase 1: 基盤 (toastBus + Backend)

- toastBus モジュール新設 (`gwt-gui/src/lib/toastBus.ts`)
- バックエンド merge_pull_request コマンド追加 (`crates/gwt-tauri/src/commands/pullrequest.rs`)
- App.svelte で toastBus を購読

### Phase 2: UIコンポーネント

- MergeConfirmModal コンポーネント新設 (`gwt-gui/src/lib/components/MergeConfirmModal.svelte`)
- PrStatusSection バッジのボタン化

### Phase 3: 統合

- WorktreeSummaryPanel でマージフロー統合
- 全体検証

## テスト

### バックエンド

- `PR_MERGE_TIMEOUT` 定数値テスト

### フロントエンド

- toastBus: subscribe/emit/unsubscribe 基本動作
- PrStatusSection: OPEN+MERGEABLE でボタン化、CLOSED/CONFLICTING でスパン維持
- MergeConfirmModal: 表示/非表示、PR情報表示、ボタンクリックコールバック
