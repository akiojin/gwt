# Plan: Agent Canvas 画像ビューアタイル (#1769)

## 概要

Agent Canvas に画像ビューアタイルを追加する。ローカルファイル・クリップボード・URL・エージェント自動生成の4入力方式をサポートし、タイル内でズーム・パン操作を可能にする。永続化はパス参照方式とし、クリップボード画像は `~/.gwt/images/` に保存する。worktree タイルとの手動 relation edge をサポートする。

## アーキテクチャ判断

### タイルシステム拡張

- `AgentCanvasCardType` に `"image"` を追加
- `AgentCanvasCard` union に `AgentCanvasImageCard` を追加
- 既存の `buildAgentCanvasGraph` はセッションカードの自動生成ロジックであり、画像タイルはユーザー操作で追加されるため `AgentCanvasGraph` には含めず、`AgentCanvasState.cards` に直接追加する

### 画像読み込み経路

- **ローカルファイル**: Tauri コマンド（`read_image_file`）で Base64 を返す。CSP `default-src 'self'` 制約のため `convertFileSrc` / asset プロトコルは使わず、Base64 data URL 方式を採用する
- **クリップボード**: Tauri コマンド（`paste_image_from_clipboard`）で画像バイナリを取得し `~/.gwt/images/{uuid}.png` に保存、パスを返す
- **URL**: Tauri コマンド（`fetch_image_url`）でバックエンドから HTTP 取得し Base64 を返す（CSP により frontend から外部 fetch 不可のため）
- **エージェント自動生成**: ファイルパスで参照（ローカルファイルと同じ経路）

### 永続化

- `AgentCanvasPersistedState` に `imageCards: StoredImageCard[]` を追加
- `StoredImageCard` にはタイル ID・ファイルパス・レイアウト情報を保持
- クリップボード画像のみ `~/.gwt/images/` にコピー保存し、そのパスを永続化

### ズーム・パン

- タイル内の `<img>` に対して CSS `transform: scale() translate()` で実装
- ホイールでズーム、ドラッグでパン（既存のキャンバスドラッグと競合しないよう、画像タイル内ではイベント伝搬を止める）
- ダブルクリックでズームリセット（FR-5）

### Tauri capability 更新

- `fs:allow-read` を capability に追加（画像ファイル読み取り用）
- CSP に `img-src 'self' data:` を追加（Base64 data URL 表示用）

## レイヤー別責務

### Rust バックエンド（gwt-tauri）

- `commands/image.rs`: 画像関連 Tauri コマンド群
  - `read_image_file(path) -> Base64`: ファイル読み取り + Base64 エンコード + MIME 判定
  - `paste_image_from_clipboard() -> { path, base64 }`: クリップボード画像取得 → `~/.gwt/images/` 保存
  - `fetch_image_url(url) -> Base64`: HTTP GET → Base64 エンコード
- フォーマット検証: マジックバイトで PNG/JPG/SVG/WebP/GIF/BMP を判定
- サイズ上限: 50MB（設定変更可能にはしない、固定値）

### フロントエンド（gwt-gui）

- `agentCanvas.ts`: 型定義追加（`AgentCanvasImageCard`、`AgentCanvasCardType` 拡張）
- `ImageViewerTile.svelte`: 画像表示 + ズーム・パン UI コンポーネント
- `AgentCanvasPanel.svelte`: 画像タイル追加 UI（D&D ハンドラ、コンテキストメニュー）、画像タイルレンダリング
- `agentTabsPersistence.ts`: 画像カード永続化対応

## 依存関係

- `base64` crate（Base64 エンコード用）
- `arboard` crate（クリップボード画像取得用）— Tauri v2 は clipboard API を画像対応していないため
- `reqwest`（URL 画像取得用）— 既存依存で利用可能か確認要
- `uuid` crate（クリップボード画像ファイル名生成用）— 既存依存で利用可能か確認要

## リスク・制約

- CSP 制約により外部 URL 画像の直接表示は不可 → バックエンド経由の Base64 方式で回避
- 大画像（50MB 超）はバックエンドで拒否し、フロントエンドでエラー表示
- クリップボード API はプラットフォーム依存（macOS/Windows/Linux で挙動差がありうる）
- 画像タイルのキャンバスドラッグとタイル内パンの操作競合 → modifier key（Ctrl/Cmd）またはタイル内専用ハンドル領域で区別
