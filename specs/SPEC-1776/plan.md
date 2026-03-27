# Plan: SPEC-1776 — Electron Full Scratch Migration

## Summary

Tauri v2 (WKWebView) を Electron (Chromium) に全面置換する。
Rust バックエンドは gwt-server サイドカープロセス（axum HTTP/WS）として維持し、
gwt-core は変更なし。フロントエンドは Svelte 5 で新規作成する。

## Technical Context

### Affected Modules

| Layer | Current | After Migration |
|-------|---------|-----------------|
| Desktop Shell | Tauri v2 (WKWebView) | Electron 35+ (Chromium) |
| Backend Binary | `crates/gwt-tauri/` (161 commands) | `crates/gwt-server/` (axum HTTP/WS) |
| Core Logic | `crates/gwt-core/` | 変更なし |
| Frontend | `gwt-gui/` (Svelte 5 + Tauri IPC) | `gwt-electron/` (Svelte 5 + HTTP/WS) |
| State Mgmt | `State<AppState>` via Tauri manage | `Arc<AppState>` via axum State |
| Events | `app_handle.emit()` | WebSocket broadcast |
| Build/Dist | `cargo tauri build` | `electron-builder` |

### Proven Pattern

`crates/gwt-tauri/src/http_server.rs` に既に axum ベースの HTTP IPC サーバーが存在。
10 エンドポイントが本番稼働中。このパターンを 161 コマンド全体に拡張する。

- POST + JSON body → `spawn_blocking` → `_impl` 関数 → JSON response
- `StructuredError` → HTTP 500 + JSON エラーレスポンス
- CORS 許可 (`CorsLayer::new().allow_origin(Any)`)

### Key Dependencies (gwt-server)

- `gwt-core` (workspace) — git, PTY, AI, config, terminal
- `axum` — HTTP framework
- `tokio` (multi-thread) — async runtime
- `tokio-tungstenite` — WebSocket
- `tower-http` (cors) — CORS middleware
- `serde` / `serde_json` — serialization
- `tracing` — logging

## Constitution Check

| Rule | Status | Notes |
|------|--------|-------|
| Spec Before Implementation | PASS | spec.md 完成済み |
| Test-First Delivery | PLAN | 各 Phase で検証手順を定義 |
| No Workaround-First | PASS | WKWebView 問題の根本解決 |
| Minimal Complexity | PASS | 既存パターン拡張、gwt-core 変更なし |
| Verifiable Completion | PLAN | SC-001〜SC-007 で検証 |

## Project Structure

```text
gwt/
├── Cargo.toml              # workspace に gwt-server を追加
├── crates/
│   ├── gwt-core/           # 変更なし
│   ├── gwt-tauri/          # 維持（既存ユーザー向け、段階的廃止）
│   └── gwt-server/         # 【新規】スタンドアロン HTTP/WS サーバー
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs     # エントリーポイント、ポート出力
│           ├── server.rs   # axum ルーター構築
│           ├── state.rs    # Arc<AppState> (Tauri 非依存)
│           ├── ws.rs       # WebSocket ブロードキャスト
│           ├── error.rs    # StructuredError → HTTP レスポンス
│           └── handlers/   # コマンドハンドラ (モジュール別)
│               ├── mod.rs
│               ├── terminal.rs
│               ├── branches.rs
│               ├── git_view.rs
│               ├── project.rs
│               ├── sessions.rs
│               ├── issue.rs
│               ├── pullrequest.rs
│               ├── settings.rs
│               ├── system.rs
│               ├── assistant.rs
│               ├── cleanup.rs
│               ├── voice.rs
│               └── ...
├── gwt-electron/           # 【新規】Electron + Svelte 5 フロントエンド
│   ├── package.json
│   ├── electron/
│   │   ├── main.ts         # Electron メインプロセス
│   │   ├── preload.ts      # contextBridge API
│   │   ├── sidecar.ts      # Rust プロセス管理
│   │   ├── menu.ts         # ネイティブメニュー
│   │   ├── tray.ts         # システムトレイ
│   │   └── ipc.ts          # ipcMain ハンドラ
│   ├── src/
│   │   ├── App.svelte
│   │   ├── lib/
│   │   │   ├── api/
│   │   │   │   ├── client.ts    # HTTP クライアント
│   │   │   │   ├── events.ts    # WebSocket クライアント
│   │   │   │   └── types.ts     # API 型定義
│   │   │   ├── components/      # UI コンポーネント
│   │   │   ├── stores/          # Svelte stores
│   │   │   └── terminal/        # xterm.js ラッパー
│   │   └── app.css
│   ├── electron-builder.config.js
│   └── vite.config.ts
└── specs/SPEC-1776/
```

## Complexity Tracking

| Risk | Impact | Mitigation |
|------|--------|------------|
| 161 コマンドの移植量 | HIGH | 既存 `_impl` 関数をそのまま呼び出し。機械的作業 |
| WebSocket ターミナル出力 | MEDIUM | バイナリフレームで PTY バイト列を直接配信 |
| AppState の Tauri 非依存化 | MEDIUM | 既に `AppState::new()` は Tauri 非依存。emit() の置換のみ |
| Electron パッケージサイズ | LOW | Rust バイナリ + Chromium で 150-200MB 程度 |
| whisper-rs ネイティブビルド | LOW | gwt-server に同梱。既存ビルドフローを流用 |

## Phased Implementation

### Phase 1: gwt-server クレート基盤 (3 days)

**目的**: Tauri 非依存の axum サーバーを起動できる状態にする。

1. `crates/gwt-server/Cargo.toml` 作成
2. `state.rs`: `AppState` を Tauri 非依存で構築 (`Arc<AppState>`)
   - `crates/gwt-tauri/src/state.rs` から Tauri 型参照を除去した版
   - `app_handle.emit()` → `EventBroadcaster` trait に抽象化
3. `ws.rs`: `tokio::sync::broadcast` ベースのイベントブロードキャスター
4. `error.rs`: `StructuredError` → axum レスポンス変換
5. `server.rs`: 空ルーターの構築とCORS設定
6. `main.rs`: サーバー起動、ランダムポート割当、stdout にポート出力

**検証**: `cargo run -p gwt-server` → `curl http://localhost:{port}/healthz`

### Phase 2: コマンドハンドラ移植 (5 days)

**目的**: 161 コマンド全てを HTTP エンドポイントとして動作させる。

移植優先順:
1. **terminal** (19 cmd) — PTY 管理 + WebSocket 出力。最重要・最複雑
2. **branches** (9 cmd) — ブランチ/ワークツリー操作
3. **git_view** (8 cmd) — 差分・コミット表示
4. **project** (7 cmd) — プロジェクト管理
5. **sessions** (5 cmd) — セッション管理
6. **settings/config** (8 cmd) — 設定読み書き
7. **system** (8 cmd) — システム情報・診断
8. **issue/pullrequest** (33 cmd) — GitHub 操作
9. **assistant** (5 cmd) — アシスタント
10. **remaining** (59 cmd) — その他全て

各ハンドラのパターン:
```rust
async fn handle_list_terminals(
    AxumState(state): AxumState<SharedState>,
    Json(req): Json<ListTerminalsRequest>,
) -> Result<impl IntoResponse, HttpError> {
    blocking("list_terminals", move || {
        terminal::list_terminals_impl(&state.app_state, req.project_root)
    }).await
}
```

**検証**: 全エンドポイントに対する HTTP テスト

### Phase 3: Electron シェル (3 days)

**目的**: Electron メインプロセスで Rust サイドカーを管理し、ネイティブ機能を提供。

1. `electron-vite` でプロジェクト初期化
2. `sidecar.ts`: `child_process.spawn()` → stdout からポート取得 → 準備完了通知
3. `preload.ts`: `contextBridge` で API 公開 (port, version, dialog, shell)
4. `menu.ts`: `crates/gwt-tauri/src/menu.rs` のメニュー構造を移植
5. `tray.ts`: Show/Quit
6. `main.ts`: BrowserWindow 作成、サイドカー起動、終了処理

**検証**: `pnpm electron:dev` → ウィンドウ表示 → サイドカー接続成功

### Phase 4: フロントエンド新規作成 (7 days)

**目的**: Svelte 5 で UI を新規構築。Tauri 依存ゼロ。

1. `api/client.ts`: HTTP fetch クライアント (ポート解決、エラーハンドリング、スロットル)
2. `api/events.ts`: WebSocket クライアント (自動再接続、イベントディスパッチ)
3. `terminal/TerminalView.svelte`: xterm.js + WebSocket バイナリ入出力
4. `components/AgentCanvas/`: タイル表示、D&D、パン、ズーム
5. `components/Layout.svelte`: サイドバー + メインコンテンツ
6. `components/BranchBrowser/`: ブランチ一覧・詳細
7. `components/Settings/`: 設定パネル
8. `components/Assistant/`: アシスタントパネル
9. `App.svelte`: ルーティングとグローバル状態

**設計原則**:
- `$effect` 内で IPC 呼び出し禁止
- 状態更新は WebSocket イベント駆動
- API クライアントにコマンド別スロットル内蔵

**検証**: Playwright E2E テスト (Chromium)

### Phase 5: ビルド・配布 (2 days)

1. `electron-builder.config.js`: DMG/MSI/AppImage 設定
2. gwt-server バイナリを `extraResources` として同梱
3. macOS コード署名 + 公証
4. GitHub Actions ワークフロー
5. 全プラットフォームでのビルド確認

**検証**: CI ビルド → インストール → 起動 → 基本操作

## Dependencies Between Phases

```text
Phase 1 (gwt-server 基盤)
    ↓
Phase 2 (コマンドハンドラ移植)
    ↓
Phase 3 (Electron シェル) ← Phase 1 完了で開始可能（並列）
    ↓
Phase 4 (フロントエンド) ← Phase 2 + Phase 3 完了後
    ↓
Phase 5 (ビルド・配布) ← Phase 4 完了後
```

Phase 2 と Phase 3 は並列実行可能。
