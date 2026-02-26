# TDDノート: ReportDialog 再オープン時の状態残留バグ修正

## 対象

- `gwt-gui/src/lib/components/ReportDialog.svelte`
- `gwt-gui/src/lib/components/ReportDialog.test.ts`

## テスト戦略

1. 再オープン（`open=false -> true`）を明示的に再現するテストを先に追加して RED を作る。
2. 状態初期化は単一の `resetDialogState()` に集約し、網羅対象を固定する。
3. 既存仕様（mode で開始タブを決める）を回帰テストで保証する。

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

## 実行ログ（要約）

- `cd gwt-gui && pnpm test src/lib/components/ReportDialog.test.ts` : pass（26 tests）
