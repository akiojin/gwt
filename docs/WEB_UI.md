# Web UI 機能

gwt の Web UI は、ブラウザから Git ブランチ/Worktree を操作し、ローカル端末セッションを扱える軽量 UI です。Web UI は `gwt serve` で明示的に起動します（CLI 起動時の自動起動はありません）。

## 起動方法

### 1) フロントエンドのビルド

```bash
cd crates/gwt-frontend
trunk build --release
```

### 2) Web UI サーバーの起動

```bash
# バイナリがある場合
gwt serve --port 3000 --address 127.0.0.1

# ソースから起動する場合
cargo run -p gwt-cli -- serve --port 3000 --address 127.0.0.1
```

デフォルトは `http://127.0.0.1:3000` でアクセスできます。

## 機能概要

### 1. Worktrees (`/` / `/worktrees`)

- Worktree 一覧表示
- 既存ブランチの Worktree 作成
- 新規ブランチ作成 + Worktree 作成
- Worktree 削除（ブランチに紐づくもののみ）

### 2. Branches (`/branches`)

- ブランチ一覧表示
- ブランチ作成（base 指定可）
- ブランチ削除（現在ブランチは不可）

### 3. Terminal (`/terminal`)

- WebSocket 経由でローカル PTY を操作
- 入力/リサイズをリアルタイム送信

### 4. Settings (`/settings`)

- `gwt` のローカル設定を参照表示（読み取り専用）

## URL 構造

- `/` - Worktree 一覧
- `/worktrees` - Worktree 一覧
- `/branches` - ブランチ一覧
- `/terminal` - ターミナル
- `/settings` - 設定表示

## REST API

### ヘルスチェック

```http
GET /api/health
```

### Worktrees

```http
GET /api/worktrees
POST /api/worktrees
  Body: { branch: string, new_branch: boolean, base_branch?: string }
DELETE /api/worktrees/{branch}
```

### Branches

```http
GET /api/branches
POST /api/branches
  Body: { name: string, base?: string }
DELETE /api/branches/{name}
```

### Settings

```http
GET /api/settings
PUT /api/settings
  Body: { default_base_branch?: string, worktree_root?: string, protected_branches?: string[] }
```

### Sessions

```http
GET /api/sessions
```

## WebSocket

```
ws://localhost:3000/ws/terminal
```

### クライアント → サーバー

```json
{ "type": "input", "data": "ls\n" }
{ "type": "resize", "cols": 80, "rows": 24 }
```

### サーバー → クライアント

```json
{ "type": "ready", "session_id": "..." }
{ "type": "output", "data": "file1.txt\nfile2.txt\n" }
{ "type": "error", "message": "Connection lost" }
```

## 技術スタック

### バックエンド

- Axum (REST + WebSocket)
- Tokio
- portable-pty
- rust-embed

### フロントエンド

- Leptos (CSR)
- Leptos Router
- Trunk
- xterm.js + xterm-addon-fit (CDN)

## 開発

### ビルド

```bash
# Web UI (WASM)
cd crates/gwt-frontend
trunk build --release

# サーバー
cargo build -p gwt-web
```

### テスト

```bash
cargo test -p gwt-web
```

## 設計仕様

仕様は以下を参照:

- [SPEC-1d62511e/spec.md](../specs/SPEC-1d62511e/spec.md) - 全体設計とWeb UI概要
- [SPEC-8adfd99e/spec.md](../specs/SPEC-8adfd99e/spec.md) - Web UI 環境変数編集（将来機能）
