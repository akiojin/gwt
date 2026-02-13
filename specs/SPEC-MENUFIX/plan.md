# 実装計画: SPEC-MENUFIX

## フェーズ1: バックエンド修正

1. `crates/gwt-tauri/src/app.rs` に `EventTarget` のimportを追加
2. `emit_menu_action` 関数内の `emit()` を `emit_to()` に変更

## フェーズ2: フロントエンド修正

1. `gwt-gui/src/App.svelte` のメニューアクションリスナーを `getCurrentWebviewWindow().listen()` に変更

## フェーズ3: 検証

1. cargo clippy で Rust コードの検証
2. svelte-check で TypeScript/Svelte コードの検証
3. 手動テスト: 複数ウィンドウでのメニューアクション動作確認
