# 実装計画: エージェントモード（GUI版）

**仕様ID**: `SPEC-ba3f610c`  
**日付**: 2026-02-12  
**仕様書**: [spec.md](./spec.md)

## 概要

GUI版にマスターエージェントを復活させ、Tool Calling を通じてサブエージェントを制御する。  
TUI/tmux 依存は廃止し、Tauri + GUI内蔵ターミナル（PTY）で完結させる。

## 技術コンテキスト

- 言語: Rust 2021 Edition / TypeScript
- GUI: Tauri v2 + Svelte 5 + xterm.js
- バックエンド: gwt-tauri + gwt-core
- ストレージ: ファイルシステム（既存の設定/セッション保存）
- テスト: cargo test / pnpm test（必要時）

## 実装スコープ

1. Tool Calling 基盤の追加
2. マスターエージェント（GUI向け）実装
3. Agent Mode タブUI
4. 既存仕様のGUI向け更新
5. Agent Mode UIの会話表示/IME送信制御/送信中表示

## 主要コード構成

```text
crates/gwt-tauri/src/
├── agent_master.rs           # GUI版マスターエージェント
├── agent_tools.rs            # Tool Calling 定義/実行
├── commands/agent_mode.rs    # Agent Mode用Tauriコマンド
└── commands/terminal.rs      # send_keys / capture / broadcast

crates/gwt-core/src/
└── ai/client.rs              # Tool Callingレスポンスパース

gwt-gui/src/
└── lib/components/AgentModePanel.svelte
```

## 受け入れ条件

- Agent Mode タブでチャット入力ができる
- Tool Calling で `send_keys_to_pane` が実行できる
- `capture_scrollback_tail` がテキストを返せる
- IME変換中のEnterで送信されない
- 送信中にスピナーが表示される
- チャット履歴が会話形式のバブル表示になる
- 新規メッセージ追加時にチャットが最下部へ自動スクロールする

## 追補: Session Summaryタブ回帰修正（2026-02-13）

- `App.svelte` の初期タブ/再初期化から `Session Summary` を除去し、`Agent Mode` を既定タブに統一する
- ブランチ選択時の `Session Summary` 再追加ロジックを削除する
- タブ復元時の active 上書き条件を `agentMode` のみに限定する
- `Tab` 型から `summary` を除去して型レベルで再発を防止する
- `MainArea.test.ts` と `appTabs.test.ts` を追加し、タブ固定仕様をテストで担保する
