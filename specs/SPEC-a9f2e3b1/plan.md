# 実装計画: Worktree詳細 MERGE 状態判定の整理

**仕様ID**: `SPEC-a9f2e3b1`
**作成日**: 2026-02-26
**更新日**: 2026-02-27

## 概要

UNKNOWN リトライ基盤は維持しつつ、MERGE 表示の意味を `merge_ui_state` に集約する。`Blocked`（必須条件で不可）と `Checks warning`（非必須のみ失敗）を分離し、`Unknown` 表示を廃止して `Checking merge status...` に統一する。

## 採用アプローチ

最もシンプルでエレガントな解: 判定をバックエンドで一度だけ合成し、フロントはその値を優先描画する。

## 実装フロー

### Phase 1: Backend 判定集約（Rust）

- `PrStatusLiteSummary` / `PrDetailResponse` に `merge_ui_state` と `non_required_checks_warning` を追加
- `compute_merge_ui_state(...)` を導入し、以下の優先順で判定
  1. `merged` / `closed`
  2. `checking`（retrying）
  3. `blocked`（`BLOCKED` / required failure / `CHANGES_REQUESTED`）
  4. `checking`（`UNKNOWN` 系）
  5. `conflicting`
  6. `mergeable`
- `compute_non_required_checks_warning(...)` を導入し、`non-required failure && !required failure` で true
- 既存 retrying オーバーレイ時は `merge_ui_state=checking` を強制

### Phase 2: Frontend 表示統一（Svelte）

- 型定義に `MergeUiState` / `mergeUiState` / `nonRequiredChecksWarning` を追加
- `PrStatusSection.svelte`
  - `merge_ui_state` 優先の表示
  - `Unknown` 文言廃止、`checking` 文言へ統一
  - `Blocked` は主バッジ表示、補助 `merge-state-badge` では非表示
  - `Checks warning` バッジを追加
- `Sidebar.svelte`
  - PR バッジ判定を `mergeUiState` 優先に変更
  - `blocked` が `checking` に上書きされない優先順位へ修正

### Phase 3: テスト更新

- Rust unit: 判定優先順と warning 条件を追加検証
- Vitest:
  - `PrStatusSection.test.ts`（checking/blocked/warning/retrying）
  - `Sidebar.test.ts`（checking, blocked のバッジ判定）
- Playwright:
  - `pr-unknown-retry.spec.ts` の `unknown` クラス期待値を `checking` に更新

## 変更対象

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-tauri/src/commands/pullrequest.rs` | merge UI 合成判定・warning 判定・レスポンス拡張・テスト |
| `gwt-gui/src/lib/types.ts` | merge UI 状態型とレスポンスフィールド追加 |
| `gwt-gui/src/lib/components/PrStatusSection.svelte` | MERGE主バッジ判定整理、warning表示、checking表示統一 |
| `gwt-gui/src/lib/components/Sidebar.svelte` | サイドバーPRバッジ判定優先順の整理 |
| `gwt-gui/src/lib/components/PrStatusSection.test.ts` | 新表示仕様に合わせたテスト更新・追加 |
| `gwt-gui/src/lib/components/Sidebar.test.ts` | checking/blocked 判定テスト更新・追加 |
| `gwt-gui/e2e/pr-unknown-retry.spec.ts` | `unknown` -> `checking` 期待値変更 |
| `specs/SPEC-a9f2e3b1/spec.md` | 仕様本文を新判定基準に更新 |
| `specs/SPEC-a9f2e3b1/tasks.md` | 実施タスクを新仕様に合わせて更新 |

## 検証計画

- `cargo test -p gwt-tauri pullrequest`
- `cd gwt-gui && pnpm test src/lib/components/PrStatusSection.test.ts src/lib/components/Sidebar.test.ts`
- `cd gwt-gui && pnpm exec playwright test e2e/pr-unknown-retry.spec.ts`
