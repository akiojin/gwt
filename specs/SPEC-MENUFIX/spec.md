# SPEC-MENUFIX: メニューアクションをフォーカスウィンドウにのみスコープする

## ステータス

- 状態: 実装中
- 種別: fix

## 概要

複数ウィンドウ環境でメニューアクション（About、Settings等）が全ウィンドウに伝播するバグを修正し、フォーカスウィンドウのみにスコープする。

## ユーザーシナリオ

### シナリオ1: 複数ウィンドウでのAboutメニュー操作

1. ユーザーがウィンドウAとウィンドウBを開いている
2. ウィンドウAにフォーカスし、メニューバーからAboutを選択する
3. ウィンドウAのみにAboutダイアログが表示される
4. ウィンドウBには何も表示されない

### シナリオ2: 異なるウィンドウでの連続メニュー操作

1. ウィンドウAでAboutメニューを開く → ウィンドウAにのみ表示
2. ウィンドウBにフォーカスを移し、Settingsメニューを開く → ウィンドウBにのみ表示

## 受け入れシナリオ

- AC-1: メニューアクションイベントがフォーカスウィンドウにのみ送信されること
- AC-2: フロントエンドリスナーが自ウィンドウ宛のイベントのみを受信すること
- AC-3: 他のウィンドウでダイアログが表示されないこと
- AC-4: ターミナル出力等の全ウィンドウブロードキャストイベントに影響がないこと

## 機能要件

- FR-1: バックエンドの `emit_menu_action` は `emit_to(EventTarget::webview_window(label))` を使用してイベントを送信する
- FR-2: フロントエンドのメニューアクションリスナーは `getCurrentWebviewWindow().listen()` を使用する
- FR-3: メニューアクション以外のイベント（ターミナル出力等）のブロードキャスト動作は変更しない

## 技術設計

### バックエンド変更 (crates/gwt-tauri/src/app.rs)

- `emit()` を `emit_to(EventTarget::webview_window(window.label()))` に変更
- `EventTarget` をimportに追加

### フロントエンド変更 (gwt-gui/src/App.svelte)

- `@tauri-apps/api/event` の `listen()` を `@tauri-apps/api/webviewWindow` の `getCurrentWebviewWindow().listen()` に変更

## 成功基準

1. 複数ウィンドウ環境でメニューアクションがフォーカスウィンドウにのみ適用される
2. cargo clippy がエラーなしでパスする
3. svelte-check がエラーなしでパスする
4. 既存のテストがすべてパスする
