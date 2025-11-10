# クイックスタートガイド: Web UI機能

**日付**: 2025-11-10
**仕様ID**: SPEC-d5e56259
**対象読者**: claude-worktree開発者

## 概要

このガイドは、claude-worktree Web UI機能の開発環境セットアップから実行までの手順を説明します。

## 前提条件

- **Bun**: 1.0以上（ローカル開発）
- **Node.js**: 18以上（CI/CDで使用）
- **Git**: 2.30以上
- **OS**: Linux / macOS / Windows 10 1809以降（ConPTY対応）

## セットアップ

### 1. リポジトリのクローン

```bash
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree
```

### 2. 依存関係のインストール

```bash
bun install
```

**インストールされるパッケージ（主要）**:
- 既存: React 19, Ink 6, Vitest, execa
- 新規: Fastify 5, @fastify/websocket, node-pty, xterm.js

### 3. プロジェクト構造の確認

```bash
tree -L 2 src/
```

**期待される構造**:
```
src/
├── cli/              # CLI UI（既存のInk実装）
│   └── ui/
├── web/              # Web UI（新規）
│   ├── server/       # Fastifyサーバー
│   └── client/       # Vite + React
├── core/             # 共通ビジネスロジック
│   ├── git.ts
│   ├── worktree.ts
│   └── ...
└── index.ts          # エントリーポイント
```

## 開発ワークフロー

### CLI開発（既存）

#### ビルド

```bash
bun run build
```

**出力**: `dist/index.js`

#### 実行

```bash
# グローバルインストール
bun add -g .
claude-worktree

# または直接実行
bunx .
```

#### テスト

```bash
bun test
```

### Web UI開発（新規）

#### バックエンド開発

**開発サーバー起動**:
```bash
bun run dev:server
```

**出力**:
```
[fastify] Server listening at http://localhost:3000
[pty] PTY manager initialized
```

**ホットリロード**: ファイル変更時に自動再起動

**ログ確認**:
```bash
# REST APIログ
curl http://localhost:3000/api/health

# WebSocketログ
wscat -c ws://localhost:3000/ws/terminal/<sessionId>
```

#### フロントエンド開発

**開発サーバー起動**:
```bash
bun run dev:client
```

**出力**:
```
[vite] dev server running at http://localhost:5173
[vite] ready in 234ms
```

**ホットリロード**: HMR（Hot Module Replacement）有効

**ブラウザアクセス**:
```
http://localhost:5173
```

#### 同時起動（推奨）

```bash
# ターミナル1: バックエンド
bun run dev:server

# ターミナル2: フロントエンド
bun run dev:client
```

**または concurrently を使用**:
```bash
bun run dev:web
```

### フルビルド

#### 1. バックエンドビルド

```bash
bun run build:server
```

**出力**: `dist/web/server/`

#### 2. フロントエンドビルド

```bash
bun run build:client
```

**出力**: `dist/web/client/`（静的ファイル）

#### 3. 一括ビルド

```bash
bun run build:web
```

**出力**:
```
dist/
├── index.js          # CLI
├── web/
│   ├── server/       # バックエンド
│   └── client/       # フロントエンド（静的ファイル）
```

## よくある操作

### Web UIサーバー起動

```bash
# 開発ビルドで起動
bunx . serve

# または
bun run start:web
```

**出力**:
```
[claude-worktree] Web UI server starting...
[fastify] Server listening at http://localhost:3000
[info] Open http://localhost:3000 in your browser
```

### ブランチ一覧取得（REST API）

```bash
curl http://localhost:3000/api/branches
```

**レスポンス**:
```json
{
  "success": true,
  "data": [
    {
      "name": "main",
      "type": "local",
      "commitHash": "a1b2c3d",
      "mergeStatus": "unmerged",
      "worktreePath": null
    },
    ...
  ]
}
```

### Worktree作成（REST API）

```bash
curl -X POST http://localhost:3000/api/worktrees \
  -H "Content-Type: application/json" \
  -d '{"branchName": "feature/test"}'
```

**レスポンス**:
```json
{
  "success": true,
  "data": {
    "path": "/claude-worktree/.worktrees/feature-test",
    "branchName": "feature/test",
    "head": "a1b2c3d",
    "isLocked": false,
    "isPrunable": false
  }
}
```

### AI Toolセッション開始（REST API）

```bash
curl -X POST http://localhost:3000/api/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "toolType": "claude-code",
    "mode": "normal",
    "worktreePath": "/claude-worktree/.worktrees/feature-test"
  }'
```

**レスポンス**:
```json
{
  "success": true,
  "data": {
    "sessionId": "550e8400-e29b-41d4-a716-446655440000",
    "toolType": "claude-code",
    "mode": "normal",
    "status": "running",
    "ptyPid": 12345,
    "websocketId": "ws-abc123"
  }
}
```

### WebSocket接続（ターミナル操作）

```bash
# wscat（WebSocketクライアント）をインストール
npm install -g wscat

# WebSocket接続
wscat -c ws://localhost:3000/ws/terminal/550e8400-e29b-41d4-a716-446655440000

# 入力送信
> {"type":"input","data":"ls -la\n"}

# 出力受信
< {"type":"output","data":"total 120\ndrwxr-xr-x ..."}
```

## トラブルシューティング

### ポート競合

**問題**: `Error: listen EADDRINUSE: address already in use :::3000`

**解決策**:
```bash
# ポート使用中のプロセスを確認
lsof -i :3000

# プロセスを終了
kill -9 <PID>

# または別のポートを使用
PORT=3001 bunx . serve
```

### PTY初期化エラー（Windows）

**問題**: `Error: Cannot find module 'node-pty'`

**原因**: node-ptyのネイティブビルド失敗（Windows）

**解決策**:
```bash
# ビルドツールをインストール
npm install --global windows-build-tools

# node-ptyを再インストール
bun remove node-pty
bun add node-pty
```

**または prebuilt版を使用**:
```bash
bun add node-pty-prebuilt-multiarch
```

### WebSocket接続エラー

**問題**: `WebSocket connection to 'ws://localhost:3000/ws/terminal/...' failed`

**原因1**: セッションIDが無効

**解決策**:
```bash
# 有効なセッションを作成
curl -X POST http://localhost:3000/api/sessions -H "Content-Type: application/json" -d '...'

# 返却されたsessionIdを使用
```

**原因2**: サーバーが起動していない

**解決策**:
```bash
# サーバーを起動
bunx . serve
```

### ビルドエラー

**問題**: `error: Cannot find module '@fastify/websocket'`

**原因**: 依存関係がインストールされていない

**解決策**:
```bash
# 依存関係を再インストール
rm -rf node_modules bun.lockb
bun install
```

### Git操作エラー

**問題**: `fatal: Unable to create '.git/index.lock': File exists`

**原因**: 同時にGit操作を実行

**解決策**:
- 自動的にリトライされます（最大3回）
- それでも失敗する場合は `.git/index.lock` を手動削除

```bash
rm -f .git/index.lock
```

## 開発Tips

### ログレベル変更

```bash
# デバッグログ有効化
DEBUG=* bunx . serve

# Fastifyログのみ
DEBUG=fastify:* bunx . serve
```

### テスト実行

```bash
# すべてのテスト
bun test

# Web UI関連のテストのみ
bun test web/

# 特定のファイル
bun test web/server/routes/branches.test.ts

# ウォッチモード
bun test --watch
```

### E2Eテスト

```bash
# Playwrightインストール
bunx playwright install

# E2Eテスト実行
bun test:e2e

# ヘッドレスモードOFF（ブラウザ表示）
bun test:e2e --headed
```

### コード整形

```bash
# Prettier
bun run format

# ESLint
bun run lint

# 自動修正
bun run lint:fix
```

### 型チェック

```bash
# TypeScript型チェック
bun run type-check
```

## 次のステップ

1. ✅ 開発環境セットアップ完了
2. ⏭️ `/speckit.tasks` を実行して実装タスクを生成
3. ⏭️ `/speckit.implement` で実装開始（TDD）

## 参考リンク

- [spec.md](./spec.md) - 機能仕様書
- [plan.md](./plan.md) - 実装計画
- [research.md](./research.md) - 技術調査結果
- [data-model.md](./data-model.md) - データモデル
- [contracts/rest-api.yaml](./contracts/rest-api.yaml) - REST API仕様
- [contracts/websocket.md](./contracts/websocket.md) - WebSocketプロトコル仕様
