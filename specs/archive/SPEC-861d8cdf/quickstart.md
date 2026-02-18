# クイックスタート: Hook再登録の確認

**仕様ID**: `SPEC-861d8cdf`
**日付**: 2026-01-21

## 前提

- gwt がインストール済み
- Claude Code の hooks 設定が一度は承認されている

## 動作確認

1. gwt を起動する

2. settings.json 内の gwt hook が最新の gwt 実行パスで上書きされていることを確認する

## 手動リセット（トラブル時）

```bash
gwt hook uninstall
gwt hook setup
```

## 期待結果

- gwt 起動時に hook が再登録される
- 再登録に失敗しても gwt は起動し、ログに理由が残る
