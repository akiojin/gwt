# Implementation Plan: SPEC-1776 — gwt-tui 完全再構築

## Summary

gwt-tauri (Tauri v2 + Svelte 5 GUI) を gwt-tui (ratatui TUI) に置換する。以前の gwt-cli TUI (38,415行) のアーキテクチャ (Elm Architecture) と全機能を移植し、tmux 依存を内蔵 PTY + vt100 に置換する。

## Technical Context

### 参照コードベース
- **gwt-cli** (commit `becf0aab`): 38,415行の ratatui TUI アプリケーション
  - `tui/app.rs` (10,898行): Elm Architecture コア、16画面、48状態オブジェクト
  - `tui/screens/wizard.rs` (5,325行): 15ステップ起動ウィザード
  - `tui/screens/branch_list.rs` (3,990行): ブランチ一覧 + PR/エージェント状態
  - `tui/screens/settings.rs` (2,261行): 設定画面（7カテゴリ）
  - その他 20+ 画面モジュール

### 新規要素（gwt-cli にはなかったもの）
- **内蔵 PTY + vt100 レンダリング**: tmux 不要。renderer.rs (VT100→ratatui変換)
- **2層タブ構造**: メイン画面 (Agent/Shell) + 管理画面 (Branches/Issues/Settings/Logs)
- **Ctrl+G プレフィックスキー**: tmux の Ctrl+B 相当

### 維持する現行 gwt-tui コード
- `renderer.rs`: VT100 → ratatui Cell 変換
- `event.rs`: EventLoop (crossterm + PTY channel + tick)
- `input/keybind.rs`: Ctrl+G プレフィックス状態機械
- `Cargo.toml`: 依存関係

### gwt-core の活用
- `config::Settings`, `ProfilesConfig`, `AgentConfig` — 設定ロード
- `agent::AgentManager` — エージェント検出
- `agent::launch::AgentLaunchBuilder` — 起動パラメータ構築
- `agent::session_watcher` — リアルタイム状態更新
- `agent::session_store` — セッション永続化
- `agent::codex::codex_default_args()` — Codex 引数構築
- `worktree::WorktreeManager` — ワークツリー管理
- `docker::*` — Docker 検出/管理
- `git::Repository` — Git 操作
- `ai::*` — AI サマリー/クライアント
- `logging::init_logger()` — ログ初期化
- `terminal::PaneManager` — PTY ペイン管理

## Architecture

### Elm Architecture (Model/View/Update)

```text
Model: 全アプリ状態を保持する構造体
  ├── active_layer: MainScreen | ManagementScreen
  ├── session_tabs: Vec<SessionTab>      // Agent/Shell タブ
  ├── management_tab: ManagementTab      // Branches/Issues/Settings/Logs
  ├── wizard: Option<WizardState>        // オーバーレイ
  ├── error_queue: ErrorQueue            // エラースタック
  ├── progress: Option<ProgressState>    // 起動プログレス
  └── pane_manager: PaneManager          // PTY 管理

View: Model → Frame 描画
  ├── メイン画面: タブバー + PTY ターミナル + ステータスバー
  ├── 管理画面: タブバー + Branches/Issues/Settings/Logs + ステータスバー
  └── オーバーレイ: Wizard / Progress / Error / Confirm

Update: Message → Model 変更
  ├── Key/Mouse/Resize イベント → Message 変換
  ├── PTY 出力 → vt100 パーサー更新
  ├── Tick (250ms) → バックグラウンド結果ポーリング
  └── バックグラウンドチャネル → 状態更新
```

### Event Loop

```text
loop {
  terminal.draw(|f| model.view(f));

  if event::poll(100ms) {
    match event::read() {
      Key(key) → model.update(handle_key(key))
      Mouse(mouse) → model.update(handle_mouse(mouse))
      Resize(w,h) → model.update(Resize(w,h))
    }
  }

  // PTY output
  while let Ok((id, data)) = pty_rx.try_recv() {
    model.handle_pty_output(id, data);
  }

  // Tick (250ms)
  if tick_elapsed {
    model.update(Tick);
    model.apply_background_updates();
  }

  if model.should_quit { break; }
}
```

## File Structure

```text
crates/gwt-tui/src/
  main.rs                    — エントリーポイント + ログ初期化
  app.rs                     — Elm Architecture コア (Model/View/Update/EventLoop)
  model.rs                   — Model 構造体定義
  message.rs                 — Message enum 定義
  renderer.rs                — VT100→ratatui変換（維持）
  event.rs                   — イベントループ（維持）
  input/
    keybind.rs               — Ctrl+G プレフィックス（維持）
    voice.rs                 — whisper-rs 統合
  screens/
    mod.rs                   — Screen enum + 共通型
    branches.rs              — Branches タブ（gwt-cli branch_list.rs 移植）
    issues.rs                — Issues/SPECs タブ
    settings.rs              — Settings タブ（gwt-cli settings.rs 移植）
    logs.rs                  — Logs タブ（gwt-cli logs.rs 移植）
    agent_pane.rs            — Agent/Shell PTY ターミナル
    wizard.rs                — 起動ウィザード（gwt-cli wizard.rs 移植）
    git_view.rs              — Git View サブビュー
    clone_wizard.rs          — Clone ウィザード
    speckit_wizard.rs        — SpecKit ウィザード
    error.rs                 — エラー画面
    confirm.rs               — 確認ダイアログ
    environment.rs           — 環境変数編集
    profiles.rs              — プロファイル管理
    docker_progress.rs       — Docker 進捗
    service_select.rs        — Docker サービス選択
    port_select.rs           — Docker ポート選択
    migration_dialog.rs      — bare リポジトリ移行
  widgets/
    mod.rs
    progress_modal.rs        — プログレスモーダル
    tab_bar.rs               — メイン/管理画面タブバー
    status_bar.rs            — ステータスバー
    terminal_view.rs         — PTY ターミナル描画
  config/
    launch_defaults.rs       — 起動設定の永続化

## Phased Implementation (5-8 並列エージェント)

### Phase 1: Core Architecture
- app.rs + model.rs + message.rs: Elm Architecture コア
- 2層タブ構造、画面遷移、イベントループ
- PTY 統合（renderer.rs, event.rs を活用）

### Phase 2: Management Screens (並列)
- screens/branches.rs: ブランチ一覧 + Git View + Quick Start + Safety Level
- screens/settings.rs: 設定 + プロファイル + 環境変数 + カスタムエージェント
- screens/issues.rs: Issues/SPECs タブ
- screens/logs.rs: ログビューア

### Phase 3: Wizard + Agent Launch (並列)
- screens/wizard.rs: 15ステップウィザード完全移植
- screens/agent_pane.rs: PTY ターミナルエミュレーター
- widgets/progress_modal.rs: 6段階起動プログレス

### Phase 4: Additional Features (並列)
- Docker 対応 (Compose + DevContainer)
- Clone/Migration/SpecKit ウィザード
- ボイス入力 (whisper-rs)
- ファイルペースト
- Assistant Mode

### Phase 5: Polish
- パフォーマンス最適化
- マウス完全対応
- エラーハンドリング (ErrorQueue + モーダル + ステータスバー)
- スキル登録の自動注入
- npm 配布

## SPEC 更新

162 SPEC 全てに gwt-tui 移行の注釈を追加:
- 10 SPEC: deprecated (GUI固有)
- 5 SPEC: TUI 向けに更新
- 15 SPEC: バックエンドのみ（変更不要）
- 132 SPEC: 完了済み（注釈のみ）

## Verification

- `cargo build -p gwt-tui && cargo test -p gwt-tui && cargo clippy -p gwt-tui -- -D warnings`
- `cargo test -p gwt-core`
- 手動: gwt 起動 → Branches タブ → Wizard → Agent タブ → 管理画面トグル
- 全 SC-001〜SC-011 の達成確認
