# クイックスタート: エージェントモード（GUI版）

## 前提

- GUI版 gwt が起動できること
- AI設定が有効であること

## 起動

1. GUIで gwt を起動
2. タブバーの `Agent Mode` を選択
3. チャット入力欄にタスクを入力して送信

## 手動テスト

1. Agent Mode でメッセージ送信
2. `send_keys_to_pane` を含むTool Calling が実行される
3. `capture_scrollback_tail` でテキストが取得される
