# Web UI 機能

gwt のWeb UI機能により、ブラウザからGitブランチとWorktreeを管理し、AI Toolセッションを起動できます。

## 起動方法

```bash
# ビルド
bun run build

# Web UIサーバーを起動
bun run start:web

# または直接実行
bunx . serve
```

デフォルトでは `http://localhost:3000` でアクセス可能です。

## 機能概要

### 1. ブランチ一覧ページ (`/`)

- すべてのローカル・リモートブランチを表示
- マージステータス（merged/unmerged/unknown）
- Worktreeの有無を表示
- ベースブランチ推定（PR baseRef / Git upstream / merge-baseヒューリスティクス）に基づいたノードグラフ表示
- グラフではBaseノード→派生ブランチがレーン上で接続され、ホバー時にbase/divergence/worktreeパスをツールチップ表示
- ブランチ名クリックで詳細ページへ遷移

### 2. ブランチ詳細ページ (`/:branchName`)

- ブランチの詳細情報表示
  - コミットハッシュ、メッセージ、作成者、日付
  - Divergence情報（ahead/behind）
- Worktree管理
  - Worktree作成ボタン
  - Worktree削除（TODO）
- AI Toolセッション起動
  - Claude Code起動
  - Codex CLI起動
  - ブラウザ端末エミュレータ（xterm.js）でリアルタイム操作

### 3. ターミナルエミュレータ

- xterm.jsベースのブラウザ端末
- WebSocket経由でPTYプロセスと双方向通信
- ANSI escape codeのフルサポート
- ウィンドウリサイズ対応
- セッション終了時の自動クリーンアップ

## URL構造

- `/` - ブランチ一覧（ホーム）
- `/:branchName` - 個別ブランチ詳細
  - 例: `/feature-webui`
  - スラッシュを含むブランチ: `/feature%2Fwebui` (URL エンコード)

## REST API

### ヘルスチェック

```http
GET /api/health
```

### ブランチ

```http
GET /api/branches                    # 一覧取得
GET /api/branches/:branchName        # 詳細取得
```

### Worktree

```http
GET /api/worktrees                   # 一覧取得
POST /api/worktrees                  # 作成
  Body: { branchName: string, createBranch?: boolean }
DELETE /api/worktrees?path=<path>    # 削除
```

### AI Toolセッション

```http
GET /api/sessions                    # 一覧取得
GET /api/sessions/:sessionId         # 詳細取得
POST /api/sessions                   # 起動
  Body: {
    toolType: "claude-code" | "codex-cli" | "custom",
    toolName?: string,
    mode: "normal" | "continue" | "resume",
    worktreePath: string
  }
DELETE /api/sessions/:sessionId      # 終了
```

### カスタムAI Tool設定

```http
GET /api/config                      # 取得
PUT /api/config                      # 更新
  Body: { tools: CustomAITool[] }
```

## WebSocket

```
ws://localhost:3000/api/sessions/:sessionId/terminal?sessionId=<sessionId>
```

### クライアント→サーバー

```json
{ "type": "input", "data": "ls\n" }
{ "type": "resize", "data": { "cols": 80, "rows": 24 } }
{ "type": "ping" }
```

### サーバー→クライアント

```json
{ "type": "output", "data": "file1.txt\nfile2.txt\n" }
{ "type": "exit", "data": { "code": 0, "signal": null } }
{ "type": "error", "data": { "message": "Connection lost" } }
{ "type": "pong" }
```

## 技術スタック

### バックエンド

- Fastify 5.2.1 - 高速Webフレームワーク
- @fastify/websocket 11.2.0 - WebSocketサポート
- @fastify/static 8.0.3 - 静的ファイル配信
- node-pty 1.1.0-beta9 - 疑似端末管理

### フロントエンド

- React 19.2.0 - UIライブラリ
- React Router 7.2.0 - ルーティング
- Vite 6.0.7 - ビルドツール
- xterm 5.4.0-beta.37 - ターミナルエミュレータ
- xterm-addon-fit 0.9.0-beta.37 - ターミナルリサイズ
- TanStack Query 5.66.1 - データフェッチ
- Zustand 5.0.2 - 状態管理（予定）

## 開発

### ビルド

```bash
# 完全ビルド
bun run build

# CLIのみ
bun run build:cli

# Web UIのみ
bun run build:web

# クライアントのみ
bun run build:client
```

### 開発サーバー

```bash
# フロントエンド開発サーバー（HMR有効）
bun run dev:client

# バックエンド開発サーバー（watch mode）
bun run dev:server

# 両方同時起動
bun run dev:web
```

### テスト

```bash
# E2Eテスト（Playwright）
bun run test:e2e
```

## 設計仕様

詳細な設計仕様は以下を参照:

- [機能仕様](../specs/SPEC-d5e56259/spec.md)
- [データモデル](../specs/SPEC-d5e56259/data-model.md)
- [REST API仕様](../specs/SPEC-d5e56259/contracts/rest-api.yaml)
- [WebSocket仕様](../specs/SPEC-d5e56259/contracts/websocket.md)

## TODO

- [ ] UI/UXの改善（スタイリング）
- [ ] Worktree削除機能
- [ ] セッション一覧表示ページ
- [ ] E2Eテストの追加
- [ ] エラーハンドリングの強化
- [ ] 認証・認可機能（必要に応じて）
