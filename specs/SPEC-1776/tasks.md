# Tasks: SPEC-1776 — Electron Full Scratch Migration

## Phase 0: Setup

- [ ] T-001: Cargo workspace に `crates/gwt-server` クレートを追加 (`Cargo.toml` の members)
- [ ] T-002: `crates/gwt-server/Cargo.toml` 作成 (axum, tokio, gwt-core, serde, tower-http, tokio-tungstenite, tracing)
- [ ] T-003: `gwt-electron/` ディレクトリを electron-vite + Svelte 5 で初期化 (`package.json`, `vite.config.ts`, `tsconfig.json`)
- [ ] T-004: `gwt-electron/package.json` に electron, electron-builder, @xterm/xterm 等の依存を追加

## Phase 1: Foundational — gwt-server 基盤

- [ ] T-010: `crates/gwt-server/src/error.rs` — `StructuredError` → axum `IntoResponse` 変換 (`HttpError` 型)
  - 参照: `crates/gwt-tauri/src/http_server.rs` lines 108-121
- [ ] T-011: `crates/gwt-server/src/state.rs` — Tauri 非依存の `AppState` 構築
  - `crates/gwt-tauri/src/state.rs` から移植、`tauri::*` 型を除去
  - `EventBroadcaster` フィールドを追加
  - `Arc<AppState>` でラップ
- [ ] T-012: `crates/gwt-server/src/ws.rs` — WebSocket イベントブロードキャスト
  - `tokio::sync::broadcast::Sender<ServerEvent>` ベース
  - `ServerEvent { event: String, target: EventTarget, payload: Value }`
  - axum WebSocket upgrade ハンドラ (`/ws` エンドポイント)
  - テキストフレーム (JSON イベント) + バイナリフレーム (ターミナル出力) 対応
- [ ] T-013: `crates/gwt-server/src/server.rs` — axum ルーター構築
  - CORS 設定 (`tower_http::cors`)
  - `/healthz` エンドポイント
  - `/ws` WebSocket エンドポイント
  - ハンドラモジュール群のルート登録
- [ ] T-014: `crates/gwt-server/src/main.rs` — エントリーポイント
  - `AppState::new()` → `Arc<AppState>`
  - `TcpListener::bind("127.0.0.1:0")` でランダムポート
  - stdout に `GWT_SERVER_PORT={port}` を出力
  - シグナルハンドリング (SIGTERM/SIGINT → graceful shutdown)
  - tracing 初期化
- [ ] T-015: **検証** — `cargo run -p gwt-server` → `curl /healthz` → 200 OK
  - WebSocket 接続テスト (`websocat ws://localhost:{port}/ws`)

## Phase 2: US1 — コマンドハンドラ移植 (Terminal)

> User Story 1 (Desktop App Startup) + User Story 2 (Terminal Operations)

- [ ] T-020: [P] `handlers/terminal.rs` — PTY 管理コマンド (19 cmd)
  - `launch_terminal`, `spawn_shell`, `launch_agent`, `start_launch_job`, `cancel_launch_job`, `poll_launch_job`
  - `write_terminal`, `send_keys_to_pane`, `send_keys_broadcast`, `resize_terminal`
  - `close_terminal`, `list_terminals`, `terminal_ready`
  - `probe_terminal_ansi`, `capture_scrollback_tail`, `save_clipboard_image`
  - `get_captured_environment`, `is_os_env_ready`, `get_available_shells`
  - PTY 出力ループ: `app_handle.emit("terminal-output")` → `broadcaster.send()`
  - 参照: `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T-021: [P] `handlers/system.rs` — システム/診断コマンド (8 cmd)
  - `get_system_info`, `get_stats`, `get_startup_diagnostics`, `heartbeat`
  - `report_frontend_metrics`, `get_http_ipc_port` (→ 自身のポートを返す)
  - `get_current_window_label`, `try_acquire_window_restore_leader`, `release_window_restore_leader`
- [ ] T-022: **検証** — `curl POST /list_terminals` + `curl POST /launch_terminal` → PTY 出力が WS で受信

## Phase 2: US1 — コマンドハンドラ移植 (Git/Branch)

- [ ] T-030: [P] `handlers/branches.rs` — ブランチ/ワークツリーコマンド (9 cmd)
  - `list_branches`, `list_branch_inventory`, `get_branch_inventory_detail`
  - `list_worktree_branches`, `list_remote_branches`, `materialize_worktree_ref`
  - `get_current_branch`, `list_worktrees`
  - 参照: `crates/gwt-tauri/src/commands/branches.rs`
- [ ] T-031: [P] `handlers/git_view.rs` — Git 差分/コミットコマンド (8 cmd)
  - `get_git_change_summary`, `get_branch_diff_files`, `get_file_diff`
  - `get_branch_commits`, `get_working_tree_status`, `get_stash_list`
  - `get_base_branch_candidates`
  - 参照: `crates/gwt-tauri/src/commands/git_view.rs`
- [ ] T-032: **検証** — ブランチ一覧取得、差分取得の HTTP テスト

## Phase 2: US1 — コマンドハンドラ移植 (残り全て)

- [ ] T-040: [P] `handlers/project.rs` — プロジェクト管理 (9 cmd)
  - `probe_path`, `open_project`, `get_project_info`, `close_project`
  - `is_git_repo`, `create_project`, `start_migration_job`, `quit_app`, `cancel_quit_confirm`
- [ ] T-041: [P] `handlers/sessions.rs` — セッション管理 (5 cmd)
  - `get_branch_quick_start`, `get_agent_sidebar_view`, `get_branch_session_summary`
  - `rebuild_all_branch_session_summaries`, `set_branch_display_name`
- [ ] T-042: [P] `handlers/settings.rs` — 設定/構成 (8 cmd)
  - `get_settings`, `save_settings`, `get_agent_config`, `save_agent_config`
  - `get_profiles`, `save_profiles`, `list_ai_models`
  - `get_skill_registration_status_cmd`, `repair_skill_registration_cmd`
- [ ] T-043: [P] `handlers/issue.rs` — Issue/SPEC 管理 (33 cmd)
  - issue_spec (10), local_spec (11), issue (13) の全コマンド
  - 参照: `crates/gwt-tauri/src/commands/issue_spec.rs`, `local_spec.rs`, `issue.rs`
- [ ] T-044: [P] `handlers/pullrequest.rs` — PR 操作 (12 cmd)
  - `fetch_pr_status`, `fetch_pr_detail`, `merge_pull_request` 等全コマンド
- [ ] T-045: [P] `handlers/assistant.rs` — アシスタント (5 cmd)
  - `assistant_get_state`, `assistant_send_message`, `assistant_start`, `assistant_stop`, `assistant_get_dashboard`
- [ ] T-046: [P] `handlers/cleanup.rs` — クリーンアップ (8 cmd)
  - `list_worktrees`, `check_gh_available`, `cleanup_worktrees` 等
- [ ] T-047: [P] `handlers/voice.rs` — 音声 (4 cmd)
  - `get_voice_capability`, `prepare_voice_model`, `ensure_voice_runtime`, `transcribe_voice_audio`
- [ ] T-048: [P] `handlers/misc.rs` — その他 (残りの cmd)
  - `detect_agents`, `list_agent_versions`, `detect_docker_context`
  - `suggest_branch_name`, `is_ai_configured`
  - `read_recent_logs`, `get_report_system_info`, `detect_report_target`, `create_github_issue`
  - `list_project_versions`, `get_project_version_history`, `prefetch_version_history`
  - `get_recent_projects`, `sync_window_agent_tabs`, `sync_window_active_tab`
  - `check_and_fix_agent_instruction_docs`, `project_index` 系 (8 cmd)
- [ ] T-049: `server.rs` に全ハンドラモジュールのルートを登録
- [ ] T-050: **検証** — 全 161 エンドポイントに対する HTTP スモークテスト (スクリプト)

## Phase 3: US1 — Electron シェル

> User Story 1 (Desktop App Startup)

- [ ] T-060: `electron/sidecar.ts` — Rust サイドカー管理
  - `child_process.spawn()` で gwt-server を起動
  - stdout パースで `GWT_SERVER_PORT=` からポート取得
  - プロセス終了検知 → エラーダイアログ → 再起動オプション
  - アプリ終了時に `kill()` でクリーンアップ
- [ ] T-061: `electron/preload.ts` — contextBridge API
  - `window.electronAPI.sidecarPort` (number)
  - `window.electronAPI.appVersion` (string)
  - `window.electronAPI.platform` (string)
  - `window.electronAPI.dialog.openFile()` / `showMessage()`
  - `window.electronAPI.shell.openExternal(url)`
  - `window.electronAPI.window.setTitle()` / `minimize()` / `maximize()` / `close()`
  - `window.electronAPI.onMenuAction(callback)` → unsubscribe 関数を返す
- [ ] T-062: `electron/main.ts` — メインプロセス
  - BrowserWindow 作成 (1200x800, min 800x600)
  - サイドカー起動 → ポート取得 → preload にポート注入
  - 開発: Vite dev server URL ロード / 本番: `file://` ロード
  - `app.on("window-all-closed")` → macOS 以外で quit
  - `app.on("before-quit")` → サイドカー停止
  - シングルインスタンス (`app.requestSingleInstanceLock()`)
- [ ] T-063: `electron/menu.ts` — ネイティブメニュー
  - File (Open Project, Close Project, Quit)
  - Edit (Undo, Redo, Cut, Copy, Paste, Select All)
  - Git (Branches, Pull Requests)
  - Tools (Agent Canvas, Branch Browser, Settings)
  - Window (Minimize, Zoom)
  - Help (About, Report Bug)
  - メニューアクション → renderer に IPC 送信 (`menu-action`)
- [ ] T-064: `electron/tray.ts` — システムトレイ
  - アイコン表示 (macOS: Template, other: standard)
  - コンテキストメニュー: Show / Quit
- [ ] T-065: **検証** — `pnpm electron:dev` → ウィンドウ表示、メニュー動作、サイドカー接続

## Phase 4: US2/US3/US6 — フロントエンド API 層

> User Story 2 (Terminal), User Story 3 (Canvas), User Story 6 (No IPC Loop)

- [ ] T-070: `src/lib/api/client.ts` — HTTP API クライアント
  - `invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>`
  - ポート解決: `window.electronAPI.sidecarPort`
  - エラーハンドリング: `StructuredError` パース → throw
  - コマンド別スロットル (同一コマンド 100ms デバウンス)
  - プロファイリング対応 (performance.now)
- [ ] T-071: `src/lib/api/events.ts` — WebSocket イベントクライアント
  - シングル WS 接続 (`ws://localhost:{port}/ws`)
  - 自動再接続 (exponential backoff: 500ms → 1s → 2s → 4s → max 8s)
  - テキストフレーム → JSON パース → イベント名でディスパッチ
  - バイナリフレーム → ターミナル出力ルーティング
  - `events.on(eventName, callback)` / `events.off(eventName, callback)`
- [ ] T-072: `src/lib/api/types.ts` — API 型定義
  - `TerminalInfo`, `BranchInfo`, `WorktreeInfo`, `Tab`, `AgentCanvasViewport` 等
  - 既存 `gwt-gui/src/lib/types.ts` から移植
- [ ] T-073: **検証** — `client.invoke("list_terminals")` → レスポンス取得、WS イベント受信

## Phase 4: US2 — ターミナルビュー

- [ ] T-080: `src/lib/terminal/TerminalView.svelte` — xterm.js ターミナル
  - xterm.js v6 + @xterm/addon-fit
  - WS バイナリフレームで PTY 出力受信 → `terminal.write(data)`
  - キー入力 → HTTP POST `/write_terminal` (or `/send_keys_to_pane`)
  - リサイズ → HTTP POST `/resize_terminal`
  - `$effect` 内で IPC 呼び出し禁止 (FR-016)
- [ ] T-081: **検証** — ターミナル起動 → リアルタイム出力 → キー入力 → リサイズ

## Phase 4: US3 — Agent Canvas

- [ ] T-090: `src/lib/components/AgentCanvas/Canvas.svelte` — キャンバス本体
  - Figma 風フルスクリーンキャンバス
  - `touch-action: none; user-select: none;` (D&D/パン問題防止)
  - ポインターイベントでタイル D&D
  - 背景ドラッグでパン操作
  - Ctrl+ホイールでズーム
  - CSS transform ベースのビューポート制御
- [ ] T-091: `src/lib/components/AgentCanvas/Tile.svelte` — タイルコンポーネント
  - ドラッグハンドル (`::`)
  - 種別表示 (Assistant / Worktree / Agent / Terminal)
  - 選択状態のハイライト
- [ ] T-092: `src/lib/components/AgentCanvas/Toolbar.svelte` — ツールバー
  - タイル数表示、ズームコントロール
  - `position: absolute` でキャンバス上にオーバーレイ
- [ ] T-093: **検証** — タイル D&D、パン、ズームが滑らかに動作 (Playwright E2E)

## Phase 4: US4/US5 — Branch Browser / Settings

- [ ] T-100: [P] `src/lib/components/BranchBrowser/` — ブランチブラウザ
  - ブランチ一覧 (HTTP GET `/list_branch_inventory`)
  - ブランチ詳細パネル
  - ワークツリー管理操作
- [ ] T-101: [P] `src/lib/components/Settings/` — 設定パネル
  - 一般設定、AI モデル設定、エージェント設定
  - HTTP GET/POST `/get_settings` / `/save_settings`
- [ ] T-102: [P] `src/lib/components/Sidebar/` — サイドバーナビゲーション
  - タブ切替 (Agent Canvas, Branch Browser, Settings 等)
- [ ] T-103: [P] `src/lib/components/Assistant/` — アシスタントパネル
  - アシスタント状態表示、メッセージ送受信
- [ ] T-104: `src/App.svelte` — ルートコンポーネント
  - レイアウト構成 (サイドバー + メインコンテンツ)
  - グローバル状態初期化
  - メニューアクション受信 (`window.electronAPI.onMenuAction`)
  - プロジェクト開閉フロー
- [ ] T-105: **検証** — 全画面遷移、設定読み書き、ブランチ一覧表示

## Phase 5: Polish / Cross-Cutting

- [ ] T-110: デザイントークン CSS (`app.css`)
  - 既存の `--bg-primary`, `--accent`, `--border-color` 等を移植
  - ダークモード対応
- [ ] T-111: E2E テスト (Playwright)
  - 起動テスト、ターミナル操作、Canvas D&D、ブランチブラウザ
  - `playwright.config.ts` を Electron 用に設定
- [ ] T-112: `electron-builder.config.js` — ビルド設定
  - macOS: DMG, コード署名, 公証
  - Windows: MSI (NSIS)
  - Linux: AppImage
  - gwt-server バイナリを `extraResources` に同梱
- [ ] T-113: `.github/workflows/release-electron.yml` — CI/CD
  - クロスプラットフォームビルド (macOS, Windows, Linux)
  - gwt-server のクロスコンパイルまたはマトリクスビルド
  - アーティファクトを GitHub Release にアップロード
- [ ] T-114: **最終検証** — SC-001〜SC-007 全項目の確認
  - SC-001: `pnpm electron:dev` で起動・操作可能
  - SC-002: E2E テスト全パス
  - SC-003: ターミナルリアルタイム表示
  - SC-004: Canvas D&D/パン/ズーム
  - SC-005: アイドル CPU < 5%
  - SC-006: 全プラットフォームビルド成功
  - SC-007: `@tauri-apps` import がゼロ

## Traceability Matrix

| User Story | Tasks |
|---|---|
| US1 - Desktop App Startup | T-014, T-015, T-060–T-065 |
| US2 - Terminal Operations | T-020–T-022, T-070–T-073, T-080–T-081 |
| US3 - Agent Canvas | T-090–T-093 |
| US4 - Branch Browser | T-030–T-032, T-100 |
| US5 - Settings | T-042, T-101 |
| US6 - No IPC Loop | T-070 (throttle), T-071 (event-driven), T-080 (no $effect IPC) |
