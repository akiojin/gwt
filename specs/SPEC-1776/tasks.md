# Tasks: SPEC-1776 — ratatui TUI Migration

## Phase 0: Setup

- [ ] T-001: Cargo workspace に `crates/gwt-tui` クレートを追加
- [ ] T-002: `crates/gwt-tui/Cargo.toml` 作成 (ratatui, crossterm, vt100, gwt-core, tokio)

## Phase 1: 最小 TUI シェル (US1, US3)

> User Story 1 (Launch gwt as terminal replacement) + User Story 3 (Shell tabs)

- [ ] T-010: `src/main.rs` — crossterm raw mode 開始、ratatui Terminal 初期化、パニックハンドラ
  - `crossterm::terminal::enable_raw_mode()`
  - `ratatui::Terminal::new(CrosstermBackend::new(stdout))`
  - パニック時に raw mode を確実に解除
- [ ] T-011: `src/app.rs` — App 構造体とメインイベントループ
  - `App { tabs, active_tab, mode, should_quit }`
  - `crossterm::event::poll(Duration::from_millis(10))` でイベント待機
  - `terminal.draw(|frame| ui::render(frame, &app))` で描画
- [ ] T-012: `src/pty/session.rs` — PTY セッション管理
  - `portable_pty::native_pty_system().openpty(PtySize)` で PTY 生成
  - `child_killer`, `reader`, `writer` の管理
  - PTY 出力を非同期で読み取るスレッド
  - `crates/gwt-core/src/terminal/` の既存 PTY ロジックを活用
- [ ] T-013: `src/pty/renderer.rs` — VT100 → ratatui 変換
  - `vt100::Parser` で PTY バイト列をパース
  - `vt100::Screen` → `ratatui::buffer::Buffer` のセル変換
  - ANSI カラー (256色 + TrueColor) + 属性 (bold, italic, underline) 対応
  - カーソル位置の追跡
- [ ] T-014: `src/ui/tab_bar.rs` — タブバー描画
  - タブ名、アクティブタブのハイライト、タブ番号表示
  - `ratatui::widgets::Tabs` ベース
- [ ] T-015: `src/ui/terminal.rs` — PTY 出力描画
  - `renderer.rs` の Buffer を frame に書き込み
  - カーソル表示/非表示の制御
- [ ] T-016: `src/ui/status.rs` — ステータスバー
  - 現在のタブ情報、ヘルプヒント (`Ctrl+G for commands`)
- [ ] T-017: `src/input/handler.rs` — キー入力ルーティング
  - 通常モード: キー入力を PTY に転送
  - Ctrl+G 検出 → コマンドモードに遷移
  - Ctrl+C → PTY に SIGINT 転送 (TUI は終了しない)
- [ ] T-018: **テスト** — renderer のユニットテスト
  - VT100 シーケンス → ratatui Cell 変換の正確性
  - カラーマッピング、属性、カーソル位置
- [ ] T-019: **検証** — `cargo run -p gwt-tui` → シェル操作 → ls, vim, top 等が正常動作

## Phase 2: タブ管理 + Ctrl+G (US2, US4)

> User Story 2 (Manage agent tabs) + User Story 4 (View management panel)

- [ ] T-020: `src/input/keybind.rs` — Ctrl+G プレフィックスキーシステム
  - `InputMode::Normal` / `InputMode::Command` / `InputMode::Panel` 状態マシン
  - Ctrl+G → c: 新規シェルタブ
  - Ctrl+G → n: 新規エージェント起動ダイアログ
  - Ctrl+G → 1-9: タブ切替
  - Ctrl+G → x: 現在のタブ終了
  - Ctrl+G → g: 管理パネルトグル
  - Ctrl+G → h/l: 前/次のタブ
  - Ctrl+G → q: gwt 終了
  - Ctrl+G → ?: ヘルプ表示
- [ ] T-021: `src/state/tabs.rs` — タブ状態管理
  - `Tab { id, name, tab_type, pty_session, status }`
  - `TabType::Shell | TabType::Agent { agent_id, branch, spec_id }`
  - タブ追加/削除/切替/リネーム
  - 最後のタブ終了 → gwt 終了
- [ ] T-022: `src/ui/panel.rs` — 管理パネル
  - エージェント一覧 (名前、ブランチ、ステータス)
  - 選択 → Enter でタブ切替
  - k: kill, r: restart, l: ログ表示
  - ratatui Table ウィジェット
- [ ] T-023: `src/ui/dialog.rs` — エージェント起動ダイアログ
  - エージェント種別選択 (Claude Code, Codex, Gemini)
  - ブランチ/Issue 選択
  - ディレクトリ選択
  - ratatui Popup ベース
- [ ] T-024: **テスト** — keybind のユニットテスト
  - Ctrl+G → c の状態遷移
  - Normal → Command → Normal のサイクル
  - PTY に Ctrl+G が転送されないことの検証
- [ ] T-025: **検証** — 複数タブ作成/切替/終了、Ctrl+G パネル表示

## Phase 3: エージェント起動 + gwt-core 連携 (US2, US8)

> User Story 2 (Manage agent tabs) + Acceptance Scenario 8 (worktree auto-create)

- [ ] T-030: `src/state/agents.rs` — エージェント状態管理
  - gwt-core の `PaneManager` との連携
  - エージェント起動パラメータ構築
  - ワークツリー自動作成 (gwt-core worktree モジュール)
- [ ] T-031: エージェント起動フロー統合
  - ダイアログからパラメータ受取 → worktree 作成 → PTY 起動 → タブ追加
  - gwt-core の `agent::launch` ロジック活用
- [ ] T-032: ワークツリークリーンアップ
  - タブ終了時に worktree を安全に削除
  - 未コミット変更がある場合は確認ダイアログ
- [ ] T-033: エージェント状態表示拡充
  - ステータスバーにエージェント種別、ブランチ、SPEC 表示
  - 管理パネルにリアルタイムステータス更新
- [ ] T-034: **検証** — エージェント起動 → 動作確認 → タブ終了 → worktree 消去

## Phase 4: ペイン分割 + PR/Issue パネル (US5, US6, US7)

- [ ] T-040: [P] `src/ui/split.rs` — ペイン分割
  - Ctrl+G → -: 水平分割
  - Ctrl+G → |: 垂直分割
  - Ctrl+G → 矢印キー: ペイン間移動
  - ratatui Layout::split でサイズ計算
  - 各ペインに独立した PTY セッション
- [ ] T-041: [P] `src/ui/panel.rs` — PR ダッシュボード拡充
  - PR ステータス、CI チェック結果、マージ状態
  - gwt-core の GitHub API 連携
- [ ] T-042: [P] `src/ui/panel.rs` — Issue/SPEC リスト
  - Issue 検索、SPEC 一覧、ステータス表示
- [ ] T-043: [P] AI セッションサマリー表示
  - gwt-core のセッションサマリー機能連携
  - 管理パネル内にサマリーテキスト表示
- [ ] T-044: スクロールバック実装
  - Ctrl+G → PgUp: スクロールモード開始
  - vt100 のスクロールバックバッファ活用
  - ファイル永続化 (大量出力対応)
- [ ] T-045: **検証** — ペイン分割動作、PR 表示、スクロールバック

## Phase 5: 仕上げ + 配布

- [ ] T-050: gwt-tauri / gwt-gui / gwt-server 削除
  - Cargo.toml から members 削除
  - ディレクトリ削除
  - CI 参照の更新
- [ ] T-051: CI/CD パイプライン更新
  - `.github/workflows/release.yml` を TUI バイナリ配布に変更
  - クロスプラットフォームビルド (macOS, Windows, Linux)
  - `cargo install gwt-tui` 対応
- [ ] T-052: README.md / README.ja.md 更新
  - TUI 版のインストール・使い方
  - キーバインド一覧
  - スクリーンショット
- [ ] T-053: **最終検証** — SC-001〜SC-008
  - SC-001: TUI 起動、タブバー + ターミナル表示
  - SC-002: タブ作成/切替/終了
  - SC-003: Ctrl+G パネル動作
  - SC-004: ペイン分割
  - SC-005: gwt-core テスト全パス
  - SC-006: gwt-tui テストカバレッジ > 80%
  - SC-007: gwt-tauri, gwt-gui 完全削除
  - SC-008: CI パイプライン更新

## Traceability Matrix

| User Story | Tasks |
|---|---|
| US1 - Launch as terminal replacement | T-010–T-019 |
| US2 - Manage agent tabs | T-020–T-025, T-030–T-034 |
| US3 - Shell tabs | T-012, T-021 |
| US4 - Management panel | T-022, T-023, T-025 |
| US5 - Split panes | T-040 |
| US6 - PR/Issue status | T-041, T-042 |
| US7 - AI session summaries | T-043 |
| US8 - Voice input | (P3, deferred) |
