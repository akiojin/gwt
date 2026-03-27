1. `gwt-gui/src/App.svelte` の `showReportDialog()` に `getCurrentWindow().setFocus()` を追加
2. Tauri ランタイム外での失敗は try-catch で無視（既存パターン踏襲）
3. テスト・lint・型チェック実行
