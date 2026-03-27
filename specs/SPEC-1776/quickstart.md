# Quickstart: SPEC-1776 — Electron Migration

## 最小検証フロー

### 1. gwt-server 起動確認

```bash
# ビルド
cargo build -p gwt-server

# 起動（ポート番号が stdout に出力される）
cargo run -p gwt-server
# 出力例: GWT_SERVER_PORT=54321

# ヘルスチェック
curl http://localhost:54321/healthz
# → {"status":"ok"}

# コマンド実行
curl -X POST http://localhost:54321/list_terminals \
  -H "Content-Type: application/json" \
  -d '{"projectRoot":null}'
# → []

# WebSocket 接続
websocat ws://localhost:54321/ws
# → イベントストリーム受信
```

### 2. Electron アプリ起動確認

```bash
cd gwt-electron

# 依存インストール
pnpm install

# 開発モード起動（サイドカー自動起動）
pnpm electron:dev

# 確認事項:
# - ウィンドウが表示される
# - DevTools コンソールに "Sidecar connected on port XXXXX" が出る
# - メニューバーが表示される
# - システムトレイにアイコンが表示される
```

### 3. フロントエンド基本操作確認

```
1. アプリ起動
2. プロジェクトフォルダを開く (File > Open Project)
3. Agent Canvas にタイルが表示される
4. タイルのドラッグハンドル (::) でドラッグ移動 → スムーズに移動
5. キャンバス背景をドラッグ → パン操作が動作
6. Ctrl+スクロール → ズームイン/アウト
7. エージェントを起動 → ターミナルタイルにリアルタイム出力
8. CPU 使用率 < 5% (アイドル時)
```

### 4. ビルド確認

```bash
cd gwt-electron

# プロダクションビルド
pnpm electron:build

# macOS: DMG 生成確認
ls dist/*.dmg

# インストール → 起動 → 基本操作確認
```

## トラブルシューティング

| 症状 | 原因 | 対処 |
|------|------|------|
| サイドカーが起動しない | gwt-server バイナリが見つからない | `cargo build -p gwt-server` を実行 |
| ポート接続エラー | サイドカーがクラッシュ | `~/.gwt/logs/` のログを確認 |
| WebSocket 切断 | サイドカー再起動 | 自動再接続を待つ (exponential backoff) |
| ターミナル出力なし | WS バイナリフレーム未対応 | DevTools Network タブで WS フレームを確認 |
| UI フリーズ | `$effect` ループ | DevTools Console でエラー確認、API コール頻度チェック |
