### EditMode

1. `ConsolePanel`
   - 最大行数を超えるとリングバッファとして古い行から破棄する
   - filter 切替でカテゴリ別表示が変わる
2. `LeadInputField`
   - Enter で `OnLeadCommand` が発火する
   - 空文字・空白のみは送信しない
3. `UIManager`
   - `OpenPanel` / `ClosePanel` / `CloseTopOverlay` が stack を正しく扱う
   - `OpenTerminalForAgent()` が terminal overlay を開く
4. `SettingsMenuController`
   - `OpenSettings()` で pause
   - `Resume()` で `Time.timeScale=1`
5. Localization
   - UI テキストが直接ハードコードではなくテーブル経由で引ける設計になっていること
   - **デフォルト言語が OS 言語設定に追従すること**
6. `GfmParser`（自前 GFM パーサー）
   - 見出し、段落、リスト、コードブロック、テーブル、チェックボックス、リンクが正しくパースされること
   - TextMeshPro リッチテキストに正しく変換されること
7. `NotificationService`
   - Error → トースト + コンソールに通知されること
   - Warning → コンソールのみに通知されること
   - Info → ログのみに記録されること

### PlayMode

1. HUD 常時表示
   - Lead 入力フィールドと project info が表示される
2. Overlay
   - Issue / Git / Terminal が同時表示できる
   - ドラッグ移動後も入力が壊れない
3. Focus
   - terminal focus 中は terminal input が入力を受ける
   - studio focus 中はショートカットが有効
4. Lead 質問 UI
   - world-space の `?` と選択肢 UI が表示され、回答後に閉じる
5. SPEC editor panel
   - 左チャット / 右 preview の 2 ペインが開く
6. コンテキストメニュー
   - Screen Space で描画される
