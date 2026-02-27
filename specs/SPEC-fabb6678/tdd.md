# TDDノート: ReportDialog 回帰修正（再オープン初期化 + 最前面表示）

## 対象

- `gwt-gui/src/lib/components/ReportDialog.svelte`
- `gwt-gui/src/lib/components/ReportDialog.test.ts`
- `gwt-gui/src/styles/global.css`
- `gwt-gui/src/lib/components/MigrationModal.svelte`
- `gwt-gui/src/lib/components/LaunchProgressModal.svelte`
- `gwt-gui/e2e/dialogs-common.spec.ts`

## テスト戦略

1. 再オープン（`open=false -> true`）を明示的に再現するテストを先に追加して RED を作る。
2. 状態初期化は単一の `resetDialogState()` に集約し、網羅対象を固定する。
3. 既存仕様（mode で開始タブを決める）を回帰テストで保証する。
4. モーダル競合（Migration/Launch）を E2E で再現し、Report が常に最前面で入力可能であることを保証する。

## Red / Green 記録

### T1: Bug入力の残留防止

- **Red**: 再オープン後も `Title/Steps/Expected/Actual` が残る。
- **Green**: 再オープン後は4項目すべて空文字になる。

### T2: 診断・キャプチャ状態の残留防止

- **Red**: 再オープン後も Logs/Screen Capture のチェックや Terminal Captured 表示が残る。
- **Green**: 再オープン後は `System Info=ON`, `Logs=OFF`, `Screen Capture=OFF`、Captured 表示なし。

### T3: submit失敗UIの残留防止

- **Red**: 再オープン後も submit 失敗メッセージと `Copy/Open` ボタンが残る。
- **Green**: 再オープン後は失敗メッセージとフォールバックUIが消える。

### T4: mode優先タブの保証

- **Red**: 前回選択タブが再オープン時に残る。
- **Green**: 再オープンごとに呼び出し `mode`（bug/feature）がアクティブタブになる。

### T5: Report overlay レイヤークラス保証（Unit）

- **Red**: `.report-overlay` が最前面クラスを持たない。
- **Green**: `.report-overlay` が `modal-overlay-report` を持つ。

### T6: Migration モーダル併存時の最前面保証（E2E）

- **Red**: Report の `z-index` が Migration より低く（1000 < 2000）、背面化する。
- **Green**: Report の `z-index` が Migration より高く（3000 > 2000）、`#bug-title` にフォーカス可能。

## 実行ログ（要約）

- `cd gwt-gui && pnpm test src/lib/components/ReportDialog.test.ts` : pass（27 tests）
- `cd gwt-gui && pnpm exec playwright test e2e/dialogs-common.spec.ts --project=chromium` : pass（11 tests）
