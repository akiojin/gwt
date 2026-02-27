# タスク一覧: Worktree詳細 MERGE 状態判定の整理

**仕様ID**: `SPEC-a9f2e3b1`
**作成日**: 2026-02-26
**更新日**: 2026-02-27

## タスク依存関係

```text
T001 -> T002 -> T003 -> T004
  |      |       |
  +----> T005 ---+
```

## Phase 1: Backend

### T001: MERGE UI 状態をレスポンスへ追加 (FR-001, FR-002)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`

- [x] `PrStatusLiteSummary` に `merge_ui_state`, `non_required_checks_warning` を追加
- [x] `PrDetailResponse` に同フィールドを追加
- [x] 既存シリアライズテストを新フィールド込みで更新

### T002: 判定関数の整理 (FR-003, FR-004, FR-005)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`

- [x] `compute_merge_ui_state` を導入
- [x] `blocked` を required failure / BLOCKED / changes requested で判定
- [x] `checking` を retrying または UNKNOWN 系で判定
- [x] `compute_non_required_checks_warning` を導入し、非必須のみ失敗で true
- [x] リトライ中オーバーレイで `merge_ui_state=checking` を強制

## Phase 2: Frontend

### T003: 詳細ビューの表示統一 (FR-006, FR-007, FR-008, FR-010)

**ファイル**: `gwt-gui/src/lib/components/PrStatusSection.svelte`, `gwt-gui/src/lib/types.ts`

- [x] `MergeUiState` 型を追加
- [x] `mergeUiState` / `nonRequiredChecksWarning` を型へ追加
- [x] `Unknown` 文言を廃止し `Checking merge status...` に統一
- [x] `Blocked` を主バッジ表示へ統一
- [x] `Checks warning` バッジを追加
- [x] retrying 時は checking + pulse 表示

### T004: サイドバー表示判定の同期 (FR-009, FR-011)

**ファイル**: `gwt-gui/src/lib/components/Sidebar.svelte`

- [x] `prBadgeClass` を `mergeUiState` 優先に変更
- [x] `blocked` が `checking` に潰れない優先順に修正
- [x] retrying の pulse 表示を維持

## Phase 3: テスト

### T005: テスト更新・追加 (SC-001, SC-002, SC-003, SC-004)

**ファイル**:

- `crates/gwt-tauri/src/commands/pullrequest.rs`
- `gwt-gui/src/lib/components/PrStatusSection.test.ts`
- `gwt-gui/src/lib/components/Sidebar.test.ts`
- `gwt-gui/e2e/pr-unknown-retry.spec.ts`

- [x] Rust unit: blocked 優先・warning 条件のテスト追加
- [x] PrStatusSection unit: checking/blocked/warning/retrying の期待値更新
- [x] Sidebar unit: unknown -> checking、blocked 優先のテスト更新
- [x] Playwright: `unknown` クラス期待値を `checking` へ更新
- [x] 検証コマンド実行
  - `cargo test -p gwt-tauri pullrequest`
  - `cd gwt-gui && pnpm test src/lib/components/PrStatusSection.test.ts src/lib/components/Sidebar.test.ts`
  - `cd gwt-gui && pnpm exec playwright test e2e/pr-unknown-retry.spec.ts`
