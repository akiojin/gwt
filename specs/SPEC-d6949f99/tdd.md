# TDDノート: Session Summary + PR Status Preview（GUI）

**仕様ID**: `SPEC-d6949f99`
**更新日**: 2026-02-14
**対象**:
- `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- `gwt-gui/src/lib/components/GitSection.svelte`
- `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`

## テスト方針

1. Session Summaryのタブ構成はコンポーネントテストで固定する（4タブを明示）。
2. GitタブのUX要件（初期展開・折りたたみトグル非表示）をDOM検証で固定する。
3. 変更後も既存のPR表示・Quick Start・AI Summaryロード系テストを回帰させない。

## Red / Green 記録

### T1: Session Summaryタブを4項目に拡張

- **Red**: 既存テストは `Summary` / `PR` の2タブ前提。
- **Green**: テスト期待値を `Summary / PR / AI Summary / Git` に更新し、実装を4タブ化。

### T2: Gitタブは展開固定で表示

- **Red**: `GitSection` はデフォルトで `collapsed=true` のため、Gitタブで折りたたみ表示される。
- **Green**: `GitSection` に `collapsible` / `defaultCollapsed` を追加し、Gitタブ経由では `collapsible=false` を適用。テストで `.git-body` の存在と `.collapse-icon` 非存在を検証。

### T3: 既存機能回帰なし

- **Red**: タブ責務の分離により、既存のSummary/PRテストが崩れる可能性がある。
- **Green**: `WorktreeSummaryPanel.test.ts` を含む対象テストを実行し、既存ケースがすべて通過。

## 実行ログ（要約）

- `pnpm --dir gwt-gui test -- src/lib/components/WorktreeSummaryPanel.test.ts` : pass
- `pnpm --dir gwt-gui check` : pass（既存 warning 1件のみ）

## 残課題

- `VersionHistoryPanel.svelte` の既存 `Unused CSS selector` 警告は本仕様スコープ外。
