# Tasks: SPEC-1776 — gwt-tui 完全再構築

## Phase 0: SPEC 更新

- [x] T001: SPEC-1776 の spec.md/plan.md/tasks.md を完全更新（インタビュー結果反映）
- [x] T002: 全 162 SPEC に gwt-tui 移行注釈を追加
- [x] T003: 10 GUI 固有 SPEC を deprecated マーク
- [x] T004: 5 TUI 適応 SPEC を gwt-tui 向けに更新

## Phase 1: Core Architecture (Elm Architecture)

### Model/View/Update フレームワーク

- [x] T010: [TDD] model.rs のテスト — Model 構造体、2層タブ構造、画面遷移
- [x] T011: model.rs 実装 — Model, ActiveLayer, SessionTab, ManagementTab
- [x] T012: [TDD] message.rs のテスト — Message enum, handle_key → Message 変換
- [x] T013: message.rs 実装 — 全 Message バリアント定義
- [x] T014: [TDD] app.rs のテスト — イベントループ、Model↔View↔Update サイクル
- [x] T015: app.rs 実装 — Elm Architecture コア（gwt-cli app.rs から tmux 除去して移植）
- [x] T016: main.rs 実装 — エントリーポイント + init_logger() + スキル登録

### PTY 統合

- [x] T020: [TDD] agent_pane.rs のテスト — PTY 起動、キー転送、vt100 レンダリング
- [x] T021: screens/agent_pane.rs 実装 — Agent/Shell タブの PTY ターミナルエミュレーター
- [x] T022: PTY リーダースレッド → EventLoop 統合
- [x] T023: Ctrl+C ハンドリング（Agent タブ: PTY転送、Shell タブ: 2回押し終了）
- [x] T024: ターミナルリサイズ → PaneManager + vt100 同期
- [x] T025: [TDD] app.rs / agent_pane.rs のテスト — PTY copy mode のキーボード/マウス操作、クリップボードコピー、viewport 固定
- [x] T026: PTY copy mode 実装 — `Ctrl+G,m`、動的 mouse capture、scrollback 移動、ドラッグコピー
- [x] T027: [TDD] bracketed paste のテスト — 改行を含む pasted text を PTY に一括転送
- [x] T028: bracketed paste 実装 — `Event::Paste` 配線、PTY への raw payload 転送

### Widgets

- [x] T030: [TDD] widgets/tab_bar.rs のテスト — メイン/管理画面タブバー描画
- [x] T031: widgets/tab_bar.rs 実装 — 2層タブバー（メイン: Sessions, 管理: Branches/SPECs/Issues/Versions/Settings/Logs）
- [x] T032: [TDD] widgets/status_bar.rs のテスト
- [x] T033: widgets/status_bar.rs 実装
- [x] T034: [TDD] widgets/terminal_view.rs のテスト
- [x] T035: widgets/terminal_view.rs 実装（renderer.rs を活用）
- [x] T036: [TDD] widgets/progress_modal.rs のテスト — 6段階プログレス + キャンセル
- [x] T037: widgets/progress_modal.rs 実装

### Phase 1 Verification

- [x] T040: gwt 起動 → 管理画面 Branches タブ表示
- [x] T041: Ctrl+G,Ctrl+G でメイン↔管理画面トグル
- [x] T042: Ctrl+G,c でシェルタブ作成、PTY 動作確認
- [x] T043: `cargo test -p gwt-tui && cargo test -p gwt-core` 全通過
- [x] T044: Main PTY copy mode — `Ctrl+G,m` でスクロール/コピー、終了時に live viewport 復帰

## Phase 2: Management Screens [P]

### Branches タブ (gwt-cli branch_list.rs 移植)

- [x] T100: [TDD] screens/branches.rs のテスト — ブランチ一覧表示、フィルタ、ソート
- [x] T101: screens/branches.rs 実装 — BranchItem, BranchListState, 表示/フィルタ/ソート
- [ ] T102: PR 状態統合 (gwt-core git::Repository)
- [ ] T103: エージェント状態表示（セッションファイルから）
- [x] T104: Safety Level 計算 + 表示
- [ ] T105: Git View サブビュー (diff, commits, working tree)
- [ ] T106: マルチセレクト + バッチ操作
- [ ] T107: マウスクリック/スクロール対応

### Issues/SPECs タブ

- [x] T110: [TDD] screens/issues.rs のテスト — Issue/SPEC 一覧、検索
- [x] T111: screens/issues.rs 実装 — GitHub Issue + ローカル SPEC 表示、検索
- [ ] T112: Issue → ブランチ作成 → エージェント起動フロー

### Settings タブ (gwt-cli settings.rs 移植)

- [x] T120: [TDD] screens/settings.rs のテスト — 7カテゴリ設定画面
- [x] T121: screens/settings.rs 実装 — General, Worktree, Web, Agent, CustomAgents, Environment, AISettings
- [x] T122: screens/profiles.rs — プロファイル管理（作成/編集/削除/切替）
- [x] T123: screens/environment.rs — 環境変数エディタ (KEY=VALUE)
- [x] T124: カスタムエージェント登録 (SPEC-71f2742d)

### Logs タブ (gwt-cli logs.rs 移植)

- [x] T130: [TDD] screens/logs.rs のテスト
- [x] T131: screens/logs.rs 実装 — ~/.gwt/logs/ のログ表示

### Phase 2 Verification

- [x] T140: Branches タブでブランチ一覧 + PR状態 + エージェント状態 表示
- [x] T141: Settings タブで設定変更 → 保存 → 反映
- [x] T142: Issues タブで Issue 検索 → 表示
- [x] T143: Logs タブでログ表示
- [x] T144: [TDD] 管理タブ順序を `Branches / SPECs / Issues / Versions / Settings / Logs` に固定
- [x] T145: [TDD] SPECs / Issues の詳細ビューで `SPEC-*` ディレクトリ解決と Markdown 描画を修正
- [x] T146: [TDD] Versions タブを最新 10 バージョンの range / commit count / summary preview 付き履歴表示へ更新
- [x] T147: [TDD] Logs タブを workspace JSONL + structured fields (`category` / `event` / `result` / `workspace` / `error_code`) 対応に更新
- [x] T148: [TDD] Branches タブの `All / Local / Remote` ビューフィルターが `origin/*` remote refs でも正しく切り替わるよう修正
- [x] T149: [TDD] Branches 一覧で `refs/remotes/<remote>/HEAD` 由来の `origin` alias を除外
- [x] T150: [TDD] Main PTY copy mode で file-backed ANSI transcript を読み、live parser を超える過去出力も表示
- [x] T151: [TDD] PTY 終了時に Agent/Shell タブを自動 close せず completed/error 状態のまま残し、最終 transcript を読めるように修正

## Phase 3: Wizard + Agent Launch [P]

### Wizard (gwt-cli wizard.rs 移植)

- [x] T200: [TDD] screens/wizard.rs のテスト — 15ステップ遷移、入力バリデーション
- [x] T201: WizardStep enum + WizardState 実装（15ステップ）
- [x] T202: QuickStart ステップ（前回設定の復元 FR-050）
- [x] T203: AgentSelect + ModelSelect + VersionSelect ステップ
- [x] T204: BranchAction + BranchTypeSelect + BranchNameInput ステップ
- [x] T205: ExecutionMode (Normal/Continue/Resume/Convert) ステップ
- [x] T206: SkipPermissions + ReasoningLevel + CollaborationModes ステップ
- [ ] T207: IssueSelect ステップ (GitHub Issue 連携ブランチ)
- [ ] T208: AIBranchSuggest ステップ (AI ブランチ名提案)
- [x] T209: ConvertAgentSelect + ConvertSessionSelect ステップ
- [x] T210: Wizard レンダリング（オーバーレイポップアップ）

### Agent Launch Orchestration

- [ ] T220: 6段階起動パイプライン (fetch → validate → worktree → skills → deps → launch)
  - [x] T220a: [TDD] AgentLaunchBuilder の auto_worktree テスト — worktree なし時のフォールバック確認
  - [x] T220b: [TDD] spawn_agent_session の worktree 解決テスト — branch 指定時に resolve_branch_working_dir が呼ばれることの確認
  - [x] T220c: spawn_agent_session でブランチ指定時に worktree パスを解決し、skill registration と builder の両方で使用する
  - [x] T220d: [TDD] resolve_repo_root テスト — 非gitディレクトリからbare repo自動検出
  - [x] T220e: [TDD] load_branches の bare repo 対応テスト
  - [x] T220f: main.rs に resolve_repo_root を実装、branches.rs の load_branches を bare repo 対応
- [ ] T221: キャンセル可能なバックグラウンド起動
- [ ] T222: セッション履歴の保存 (save_session_entry)
- [ ] T223: npm バージョン取得 + キャッシュ

### Phase 3 Verification

- [ ] T230: Branches → Enter → Wizard → 各ステップ → Launch → Agent タブ作成
- [ ] T231: Quick Start → ワンクリック起動
- [ ] T232: Issue 選択 → ブランチ自動作成 → エージェント起動

## Phase 4: Additional Features [P]

### Docker (gwt-tauri terminal.rs から移植)

- [ ] T300: Docker compose 検出 + サービス選択
- [ ] T301: DevContainer 検出 + 対応
- [ ] T302: docker compose up/exec/down ワークフロー
- [ ] T303: ポート競合検出 + ボリュームマウント
- [ ] T304: Docker 進捗画面 (docker_progress.rs)

### Clone/Migration/SpecKit

- [x] T310: Clone Wizard 実装
- [x] T311: Migration Dialog (bare リポジトリ移行)
- [x] T312: SpecKit Wizard 実装

### Voice Input

- [ ] T320: whisper-rs 統合（ネイティブオーディオキャプチャ + 音声認識）
- [ ] T321: ホットキー → 録音 → テキスト変換 → PTY 送信

### File Paste

- [ ] T330: クリップボードファイルペースト（専用ショートカット、OS ネイティブ API）

### Assistant Mode

- [ ] T340: LLM 会話 + タスク分割 (gwt-tauri assistant_engine.rs 移植)
- [ ] T341: タスク → エージェント振り分け

### Error Handling

- [x] T350: ErrorQueue + ErrorState 実装
- [x] T351: 重大エラー → モーダル、軽微 → ステータスバー

### Performance

- [ ] T360: フレームレート制限 (16ms target)
- [ ] T361: dirty flag + 差分更新
- [ ] T362: PTY 出力バッチング

### Session Watcher

- [ ] T370: gwt-core session_watcher の統合（リアルタイムエージェント状態更新）

### Skill Registration

- [x] T380: 起動時スキル登録自動実行（CLAUDE.md/AGENTS.md/GEMINI.md 注入）

## Phase 5: Cleanup + Release

### コード除去

- [x] T500: 現在の gwt-tui の不要コードを削除（state.rs, ui/management/*, etc.）
- [ ] T501: gwt-tauri/gwt-gui が develop に残っていれば削除

### CI/CD

- [ ] T510: CI ワークフロー更新（gwt-tui テスト + ビルド）
- [ ] T511: Release ワークフロー更新（クロスコンパイル + npm publish）

### Documentation

- [ ] T520: README.md / README.ja.md 更新
- [ ] T521: CLAUDE.md 更新

### Phase 5 Verification

- [x] T530: `cargo build -p gwt-tui` 成功
- [x] T531: `cargo test -p gwt-tui && cargo test -p gwt-core` 全通過
- [x] T532: `cargo clippy --all-targets --all-features -- -D warnings` クリーン
- [ ] T533: 手動 E2E テスト（起動 → Branches → Wizard → Agent → 管理画面 → 終了）
- [ ] T534: npm publish テスト
- [ ] T535: SC-001〜SC-011 全達成確認
