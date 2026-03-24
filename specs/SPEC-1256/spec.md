### User Scenario

1. ユーザーが gwt アプリを使用中にエラーが発生する
2. エラートーストの「Report」ボタンを押す、またはメニューから「Report Issue」を選択する
3. **期待**: アプリケーションウィンドウが OS 上で最前面に表示され、エラーリポートダイアログが即座に見える
4. **現状**: アプリケーションウィンドウが他のウィンドウの後ろに隠れている場合、ダイアログが見えない

### Acceptance Criteria

- [ ] `showReportDialog()` 呼び出し時に、Tauri ウィンドウが OS 上で最前面にフォーカスされる
- [ ] Windows / macOS / Linux いずれでも動作する（Tauri `setFocus()` API は全プラットフォーム対応）
- [ ] 既存のダイアログ表示動作（モーダルオーバーレイ、z-index: 3000）に影響しない

### Functional Requirements

- **FR-001**: `showReportDialog()` が呼ばれた時、`getCurrentWindow().setFocus()` でウィンドウを最前面に移動する
- **FR-002**: Tauri ランタイム外（テスト環境等）では `setFocus()` 失敗を無視する（既存パターン踏襲: App.svelte:1334）

### Success Criteria

- エラーリポートダイアログ表示時にウィンドウが最前面に来ること
- 既存テストが全て通ること
- lint / 型チェックにエラーがないこと
