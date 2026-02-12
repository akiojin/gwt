# 調査メモ: エージェントモード（GUI版）

## 既存実装の状況

- TUI/tmux 実装はリポジトリから削除済み
- GUI側は Tauri + Svelte + xterm.js の構成
- 既存のターミナル管理は `crates/gwt-tauri/src/commands/terminal.rs` と `gwt-core/src/terminal/` に存在

## 統合ポイント

- Tool Calling の実行は gwt-tauri 側で行うのが最小変更
- マスターエージェントは GUI向けに新規実装が必要
- Agent Mode UI は `MainArea.svelte` にタブとして追加

## 主要リスク

- Tool Calling レスポンス形式の差異
- AI設定未構成時のUX
