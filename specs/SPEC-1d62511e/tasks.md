# タスク一覧: TypeScript/Bun から Rust への完全移行

## 凡例

- `[ ]` 未着手
- `[~]` 進行中
- `[x]` 完了
- `[!]` ブロック中
- `(dep: X)` タスクXに依存

## Phase 1: 基盤構築

### 1.1 プロジェクト初期化

- [ ] **T1.1.1**: Cargo ワークスペース作成（gwt-core, gwt-cli, gwt-web, gwt-frontend）
- [ ] **T1.1.2**: gwt-core クレート初期化 (dep: T1.1.1)
- [ ] **T1.1.3**: gwt-cli クレート初期化 (dep: T1.1.1)
- [ ] **T1.1.4**: gwt-web クレート初期化 (dep: T1.1.1)
- [ ] **T1.1.5**: gwt-frontend クレート初期化（Leptos CSR） (dep: T1.1.1)
- [ ] **T1.1.6**: workspace.dependencies に共通依存関係設定 (dep: T1.1.1)
- [ ] **T1.1.7**: .gitignore, rustfmt.toml, clippy.toml, deny.toml 設定
- [ ] **T1.1.8**: MSRV設定（latest stable）

### 1.2 エラーハンドリング基盤 (gwt-core)

- [ ] **T1.2.1**: thiserror依存追加
- [ ] **T1.2.2**: GwtError enum定義（カテゴリ別細分化） (dep: T1.2.1)
  - E1xxx: Git操作エラー
  - E2xxx: Worktree操作エラー
  - E3xxx: 設定エラー
  - E4xxx: Agent起動エラー
  - E5xxx: Web APIエラー
- [ ] **T1.2.3**: エラーコードマクロ実装 (dep: T1.2.2)
- [ ] **T1.2.4**: エラーメッセージのバイナリ埋め込み (dep: T1.2.3)
- [ ] **T1.2.5**: エラーユニットテスト (dep: T1.2.2-T1.2.4)

### 1.3 Git操作モジュール (gwt-core)

- [ ] **T1.3.1**: gix (gitoxide) 依存追加
- [ ] **T1.3.2**: GitBackend trait定義（gix/external切り替え） (dep: T1.3.1)
- [ ] **T1.3.3**: `Repository::discover()` - リポジトリ検出 (dep: T1.3.2)
- [ ] **T1.3.4**: `Repository::root()` - ルートパス取得 (dep: T1.3.3)
- [ ] **T1.3.5**: `Branch::list()` - ブランチ一覧取得 (dep: T1.3.3)
- [ ] **T1.3.6**: `Branch::current()` - 現在ブランチ取得 (dep: T1.3.5)
- [ ] **T1.3.7**: `Branch::create()` - ブランチ作成 (dep: T1.3.3)
- [ ] **T1.3.8**: `Branch::delete()` - ブランチ削除 (dep: T1.3.3)
- [ ] **T1.3.9**: `Remote::list()` - リモート一覧 (dep: T1.3.3)
- [ ] **T1.3.10**: `Remote::fetch_all()` - 全リモート更新 (dep: T1.3.9)
- [ ] **T1.3.11**: `Repository::pull_fast_forward()` - Fast-Forward Pull (dep: T1.3.10)
- [ ] **T1.3.12**: `Repository::has_uncommitted_changes()` - 未コミット検出 (dep: T1.3.3)
- [ ] **T1.3.13**: `Repository::has_unpushed_commits()` - 未プッシュ検出 (dep: T1.3.3)
- [ ] **T1.3.14**: `Branch::divergence_status()` - 乖離状態検出 (dep: T1.3.10)
- [ ] **T1.3.15**: 外部gitコマンドフォールバック実装 (dep: T1.3.2)
- [ ] **T1.3.16**: Git統合テスト（一時リポジトリ） (dep: T1.3.1-T1.3.15)

### 1.4 Worktree管理モジュール (gwt-core)

- [ ] **T1.4.1**: Worktree型定義
- [ ] **T1.4.2**: `WorktreeManager::list()` - Worktree一覧 (dep: T1.3.3, T1.4.1)
- [ ] **T1.4.3**: `WorktreePath::generate()` - パス生成 (dep: T1.4.1)
- [ ] **T1.4.4**: `WorktreeManager::create()` - Worktree作成 (dep: T1.4.2, T1.4.3)
- [ ] **T1.4.5**: `WorktreeManager::remove()` - Worktree削除 (dep: T1.4.2)
- [ ] **T1.4.6**: `WorktreeManager::is_protected()` - 保護ブランチ判定 (dep: T1.4.1)
- [ ] **T1.4.7**: `WorktreeManager::repair_path()` - パス修復 (dep: T1.4.2)
- [ ] **T1.4.8**: `CleanupCandidate::detect()` - 孤立Worktree検出 (dep: T1.4.2)
- [ ] **T1.4.9**: Worktree統合テスト（一時リポジトリ） (dep: T1.4.1-T1.4.8)

### 1.5 設定管理モジュール (gwt-core)

- [ ] **T1.5.1**: figment, serde, toml依存追加
- [ ] **T1.5.2**: Settings型定義 (dep: T1.5.1)
- [ ] **T1.5.3**: TOML設定ファイル読み込み（.gwt.toml優先順） (dep: T1.5.2)
- [ ] **T1.5.4**: JSONからTOMLへの自動マイグレーション (dep: T1.5.3)
- [ ] **T1.5.5**: 環境変数サポート（GWT_*） (dep: T1.5.2)
- [ ] **T1.5.6**: Profile型定義・読み込み (dep: T1.5.3)
- [ ] **T1.5.7**: Session型定義（TOML形式保存/復元） (dep: T1.5.3)
- [ ] **T1.5.8**: 設定モジュール統合テスト (dep: T1.5.1-T1.5.7)

### 1.6 ログシステムモジュール (gwt-core)

- [ ] **T1.6.1**: tracing, tracing-subscriber, tracing-appender依存追加
- [ ] **T1.6.2**: Logger初期化（JSON Lines + span情報） (dep: T1.6.1)
- [ ] **T1.6.3**: Pino互換JSON形式出力 (dep: T1.6.2)
- [ ] **T1.6.4**: カテゴリ別ログ（category フィールド） (dep: T1.6.3)
- [ ] **T1.6.5**: ログファイルパス生成（~/.gwt/logs/{workspace}/{date}.jsonl） (dep: T1.6.1)
- [ ] **T1.6.6**: LogReader - ログ読み込み（遅延読み込み対応） (dep: T1.6.5)
- [ ] **T1.6.7**: ログローテーション（7日保持） (dep: T1.6.5)
- [ ] **T1.6.8**: ログモジュール統合テスト (dep: T1.6.1-T1.6.7)

### 1.7 ファイルロック (gwt-core)

- [ ] **T1.7.1**: fs2依存追加（flock対応）
- [ ] **T1.7.2**: WorktreeLock実装（.gwt.lock per worktree） (dep: T1.7.1)
- [ ] **T1.7.3**: LockGuard RAII実装 (dep: T1.7.2)
- [ ] **T1.7.4**: マルチインスタンス動作テスト (dep: T1.7.1-T1.7.3)

### 1.8 基本CLI (gwt-cli)

- [ ] **T1.8.1**: clap依存追加、CLI引数定義
- [ ] **T1.8.2**: --help, --version 実装 (dep: T1.8.1)
- [ ] **T1.8.3**: --debug フラグ + GWT_DEBUG環境変数 (dep: T1.8.1)
- [ ] **T1.8.4**: serve サブコマンド（ポート指定） (dep: T1.8.1)
- [ ] **T1.8.5**: git存在チェック（起動時） (dep: T1.8.1)
- [ ] **T1.8.6**: 対話モードエントリーポイント (dep: T1.8.1, T1.3.3)

## Phase 2: CLI TUI

### 2.1 TUIアプリケーション構造（Elm Architecture）

- [ ] **T2.1.1**: ratatui, crossterm依存追加
- [ ] **T2.1.2**: ratatui-async-templateパターン適用 (dep: T2.1.1)
- [ ] **T2.1.3**: Model型定義（アプリケーション状態） (dep: T2.1.2)
- [ ] **T2.1.4**: Message enum定義（全イベント） (dep: T2.1.2)
- [ ] **T2.1.5**: update()関数実装（状態遷移） (dep: T2.1.3, T2.1.4)
- [ ] **T2.1.6**: view()関数実装（描画） (dep: T2.1.3)
- [ ] **T2.1.7**: 画面スタック管理（ScreenStack + 状態保持） (dep: T2.1.3)
- [ ] **T2.1.8**: tokioイベントループ実装 (dep: T2.1.2)
- [ ] **T2.1.9**: キーボードイベントハンドリング（矢印キー、Enter、Esc、q） (dep: T2.1.8)
- [ ] **T2.1.10**: PageUp/PageDown/Home/End対応 (dep: T2.1.9)
- [ ] **T2.1.11**: Ctrl+Cハンドリング（2回押しで終了） (dep: T2.1.8)
- [ ] **T2.1.12**: 終了時クリーンアップ（孤立Worktree検出・修復） (dep: T2.1.11, T1.4.8)
- [ ] **T2.1.13**: Alternate Screen切り替え (dep: T2.1.8)

### 2.2 共通コンポーネント

- [ ] **T2.2.1**: Header コンポーネント（統計情報 + オフラインアイコン[OFFLINE]） (dep: T2.1.1)
- [ ] **T2.2.2**: Footer コンポーネント（キーバインド表示） (dep: T2.1.1)
- [ ] **T2.2.3**: ScrollableList コンポーネント（遅延読み込み対応） (dep: T2.1.1)
- [ ] **T2.2.4**: TextInput コンポーネント (dep: T2.1.1)
- [ ] **T2.2.5**: SelectInput コンポーネント (dep: T2.1.1)
- [ ] **T2.2.6**: Spinner コンポーネント（Agent待機用） (dep: T2.1.1)
- [ ] **T2.2.7**: Dialog コンポーネント (dep: T2.1.1)

### 2.3 画面実装

- [ ] **T2.3.1**: BranchListScreen - ブランチ一覧表示 (dep: T2.2.1-T2.2.3, T1.3.5)
- [ ] **T2.3.2**: BranchListScreen - Worktree状態表示 (dep: T2.3.1, T1.4.2)
- [ ] **T2.3.3**: BranchListScreen - 検索/フィルタ機能 (dep: T2.3.1, T2.2.4)
- [ ] **T2.3.4**: BranchListScreen - キーボード操作（矢印キー/Enter/Esc/q） (dep: T2.3.1)
- [ ] **T2.3.5**: WorktreeCreateScreen - ウィザード構造 (dep: T2.2.4, T2.2.5)
- [ ] **T2.3.6**: WorktreeCreateScreen - ブランチ名入力 (dep: T2.3.5)
- [ ] **T2.3.7**: WorktreeCreateScreen - ベースブランチ選択 (dep: T2.3.5)
- [ ] **T2.3.8**: WorktreeCreateScreen - 確認・実行 (dep: T2.3.5, T1.4.4)
- [ ] **T2.3.9**: ConfirmScreen - Yes/No選択 (dep: T2.2.7)
- [ ] **T2.3.10**: ErrorScreen - エラーコード表示 (dep: T2.2.7, T1.2.2)
- [ ] **T2.3.11**: InputScreen - テキスト入力 (dep: T2.2.4)
- [ ] **T2.3.12**: SelectorScreen - 選択肢提示 (dep: T2.2.5)
- [ ] **T2.3.13**: ProfileScreen - プロファイル管理 (dep: T2.2.3, T1.5.6)
- [ ] **T2.3.14**: EnvironmentScreen - 環境変数管理 (dep: T2.2.4)
- [ ] **T2.3.15**: SettingsScreen - 設定画面 (dep: T2.2.4, T1.5.3)
- [ ] **T2.3.16**: LogScreen - ログ一覧（遅延読み込み） (dep: T2.2.3, T1.6.6)
- [ ] **T2.3.17**: LogDetailScreen - ログ詳細 (dep: T2.3.16)
- [ ] **T2.3.18**: HelpOverlay - ヘルプ表示（h/?キー） (dep: T2.1.1)

### 2.4 オフライン対応

- [ ] **T2.4.1**: ネットワーク状態検出 (dep: T2.1.1)
- [ ] **T2.4.2**: オフライン時のgraceful degradation (dep: T2.4.1)
- [ ] **T2.4.3**: Header [OFFLINE] アイコン表示 (dep: T2.2.1, T2.4.1)

### 2.5 TUI統合テスト

- [ ] **T2.5.1**: ブランチ選択→Worktree作成フロー (dep: T2.3.1-T2.3.8)
- [ ] **T2.5.2**: Worktree削除フロー (dep: T2.3.9, T1.4.5)
- [ ] **T2.5.3**: 検索・フィルタ動作確認 (dep: T2.3.3)
- [ ] **T2.5.4**: Ctrl+C 2回終了テスト (dep: T2.1.11)

## Phase 3: Coding Agent統合

### 3.1 Agent共通基盤

- [ ] **T3.1.1**: CodingAgent トレイト定義（外部プロセス起動）
- [ ] **T3.1.2**: AgentConfig 型定義（環境変数、作業ディレクトリ）
- [ ] **T3.1.3**: プロセス起動ユーティリティ（std::process::Command）
- [ ] **T3.1.4**: Agent待機中のブロッキングSpinner表示 (dep: T2.2.6)

### 3.2 Claude Code

- [ ] **T3.2.1**: ClaudeCode 構造体・ClaudeMode enum (dep: T3.1.1)
- [ ] **T3.2.2**: Normal モード起動 (dep: T3.2.1, T3.1.3)
- [ ] **T3.2.3**: Continue モード起動 (dep: T3.2.2)
- [ ] **T3.2.4**: Resume モード起動（セッションID指定） (dep: T3.2.2)
- [ ] **T3.2.5**: セッションID保存（TOML） (dep: T3.2.2, T1.5.7)

### 3.3 Codex CLI

- [ ] **T3.3.1**: CodexCli 構造体 (dep: T3.1.1)
- [ ] **T3.3.2**: 推論レベル設定（low/medium/high） (dep: T3.3.1)
- [ ] **T3.3.3**: 起動実装 (dep: T3.3.1, T3.1.3)

### 3.4 Gemini CLI

- [ ] **T3.4.1**: GeminiCli 構造体 (dep: T3.1.1)
- [ ] **T3.4.2**: 起動実装 (dep: T3.4.1, T3.1.3)

### 3.5 Agent選択UI

- [ ] **T3.5.1**: Agent選択画面 (dep: T2.2.5, T3.2.1, T3.3.1, T3.4.1)
- [ ] **T3.5.2**: TUIからのAgent起動統合 (dep: T3.5.1, T2.3.8)

## Phase 4: Web UI

### 4.1 Axumサーバー (gwt-web)

- [ ] **T4.1.1**: axum, tower, tower-http依存追加
- [ ] **T4.1.2**: WASM埋め込み設定（rust-embed） (dep: T4.1.1)
- [ ] **T4.1.3**: 静的ファイル配信（WASMバンドル） (dep: T4.1.2)
- [ ] **T4.1.4**: CORS設定（localhost限定） (dep: T4.1.1)
- [ ] **T4.1.5**: エラーハンドリング (dep: T4.1.1, T1.2.2)

### 4.2 REST API

- [ ] **T4.2.1**: GET /api/worktrees - Worktree一覧 (dep: T4.1.1, T1.4.2)
- [ ] **T4.2.2**: POST /api/worktrees - Worktree作成 (dep: T4.1.1, T1.4.4)
- [ ] **T4.2.3**: DELETE /api/worktrees/:id - Worktree削除 (dep: T4.1.1, T1.4.5)
- [ ] **T4.2.4**: GET /api/branches - ブランチ一覧 (dep: T4.1.1, T1.3.5)
- [ ] **T4.2.5**: GET /api/sessions - セッション履歴 (dep: T4.1.1, T1.5.7)
- [ ] **T4.2.6**: GET /api/config - 設定取得 (dep: T4.1.1, T1.5.3)
- [ ] **T4.2.7**: PUT /api/config - 設定更新 (dep: T4.1.1, T1.5.3)

### 4.3 WebSocket

- [ ] **T4.3.1**: WebSocketルート追加 (dep: T4.1.1)
- [ ] **T4.3.2**: PTYマネージャー (dep: T4.3.1)
- [ ] **T4.3.3**: 端末入出力転送 (dep: T4.3.2)

### 4.4 Leptosフロントエンド (gwt-frontend)

- [ ] **T4.4.1**: Leptos CSR依存追加、trunk設定
- [ ] **T4.4.2**: ルーティング設定（leptos_router） (dep: T4.4.1)
- [ ] **T4.4.3**: API クライアント（gloo-net） (dep: T4.4.1)
- [ ] **T4.4.4**: Worktree一覧ページ (dep: T4.4.2, T4.4.3)
- [ ] **T4.4.5**: ブランチ一覧ページ (dep: T4.4.2, T4.4.3)
- [ ] **T4.4.6**: 端末ページ（xterm.js統合） (dep: T4.4.2, T4.3.3)
- [ ] **T4.4.7**: 設定ページ (dep: T4.4.2, T4.4.3)
- [ ] **T4.4.8**: スタイリング（Tailwind CSS） (dep: T4.4.1)
- [ ] **T4.4.9**: WASM最適化（wasm-opt） (dep: T4.4.1)

### 4.5 ビルド統合

- [ ] **T4.5.1**: trunk build → rust-embed連携 (dep: T4.4.1, T4.1.2)
- [ ] **T4.5.2**: 単一バイナリへのWASM埋め込み (dep: T4.5.1)

## Phase 5: 品質・配布

### 5.1 テスト整備

- [ ] **T5.1.1**: 統合テスト（Git操作フロー）- 一時リポジトリ使用
- [ ] **T5.1.2**: 統合テスト（Worktree操作フロー）- 一時リポジトリ使用
- [ ] **T5.1.3**: 統合テスト（設定読み込みフロー）
- [ ] **T5.1.4**: 統合テスト（TOML自動マイグレーション）
- [ ] **T5.1.5**: E2Eテスト（CLI操作）
- [ ] **T5.1.6**: E2Eテスト（Web API）

### 5.2 ベンチマーク

- [ ] **T5.2.1**: criterion依存追加
- [ ] **T5.2.2**: Git操作ベンチマーク (dep: T5.2.1)
- [ ] **T5.2.3**: Worktree操作ベンチマーク (dep: T5.2.1)
- [ ] **T5.2.4**: 設定読み込みベンチマーク (dep: T5.2.1)

### 5.3 CI/CD

- [ ] **T5.3.1**: GitHub Actions ワークフロー作成
- [ ] **T5.3.2**: テスト自動実行（各OS native runner） (dep: T5.3.1)
- [ ] **T5.3.3**: Clippy/Rustfmt チェック (dep: T5.3.1)
- [ ] **T5.3.4**: cargo-deny チェック (dep: T5.3.1)
- [ ] **T5.3.5**: マルチプラットフォームビルド（native runners） (dep: T5.3.1)
- [ ] **T5.3.6**: リリースワークフロー（タグトリガー） (dep: T5.3.5)

### 5.4 配布

- [ ] **T5.4.1**: Linux x86_64 ビルド確認
- [ ] **T5.4.2**: Linux aarch64 ビルド確認
- [ ] **T5.4.3**: macOS x86_64 ビルド確認
- [ ] **T5.4.4**: macOS aarch64 ビルド確認
- [ ] **T5.4.5**: Windows x86_64 ビルド確認
- [ ] **T5.4.6**: GitHub Releasesへのバイナリアップロード自動化 (dep: T5.3.6)
- [ ] **T5.4.7**: Homebrew Formula作成
- [ ] **T5.4.8**: crates.io公開設定
- [ ] **T5.4.9**: npm postinstallパッケージ作成（GH Releasesからダウンロード）

### 5.5 ドキュメント

- [ ] **T5.5.1**: README.md 更新（インストール方法）
- [ ] **T5.5.2**: CHANGELOG.md 更新
- [ ] **T5.5.3**: 移行ガイド作成（JSON→TOML設定移行）

## 依存関係サマリー

```text
Phase 1 (基盤)
├── T1.1.* プロジェクト初期化
├── T1.2.* エラーハンドリング ← T1.1.*
├── T1.3.* Git操作 ← T1.1.*, T1.2.*
├── T1.4.* Worktree ← T1.3.*
├── T1.5.* 設定 ← T1.1.*
├── T1.6.* ログ ← T1.1.*
├── T1.7.* ファイルロック ← T1.1.*
└── T1.8.* 基本CLI ← T1.3.*, T1.5.*, T1.6.*, T1.7.*

Phase 2 (TUI)
├── T2.1.* Elm Architecture ← Phase 1
├── T2.2.* コンポーネント ← T2.1.*
├── T2.3.* 画面 ← T2.2.*, Phase 1
├── T2.4.* オフライン対応 ← T2.1.*, T2.2.*
└── T2.5.* TUI統合テスト ← T2.3.*, T2.4.*

Phase 3 (Agent)
├── T3.1.* 共通基盤 ← Phase 1
├── T3.2-4.* 各Agent ← T3.1.*
└── T3.5.* Agent選択UI ← T3.2-4.*, Phase 2

Phase 4 (Web)
├── T4.1.* Axumサーバー ← Phase 1
├── T4.2.* API ← T4.1.*, Phase 1
├── T4.3.* WebSocket ← T4.1.*
├── T4.4.* Leptos Frontend ← T4.2.*, T4.3.*
└── T4.5.* ビルド統合 ← T4.4.*

Phase 5 (品質・配布)
├── T5.1.* 統合テスト ← Phase 1-4
├── T5.2.* ベンチマーク ← Phase 1
├── T5.3.* CI/CD ← T5.1.*, T5.2.*
├── T5.4.* 配布 ← T5.3.*
└── T5.5.* ドキュメント ← T5.4.*
```

## 技術決定サマリー

| 項目 | 決定事項 |
| ---- | -------- |
| Gitライブラリ | gitoxide (gix) + 外部gitフォールバック |
| TUI | ratatui + Elm Architecture |
| 状態管理 | Screen Stack（状態保持） |
| エラー | thiserror + カテゴリ別エラーコード |
| 設定形式 | TOML（JSONから自動移行） |
| ログ | JSON Lines + tracing spans |
| ロック | flock per worktree |
| キーバインド | 矢印キー、Enter、Esc、q、PageUp/Down |
| Ctrl+C | 2回押しで終了 |
| オフライン | graceful degradation + [OFFLINE]アイコン |
| Web Backend | Axum |
| Web Frontend | Leptos CSR（WASM埋め込み） |
| テスト | 統合テスト中心（一時リポジトリ） |
| ベンチマーク | criterion |
| 配布 | GitHub Releases, Homebrew, crates.io, npm |

## 進捗トラッキング

| Phase | 総タスク | 完了 | 進捗率 |
| ----- | -------- | ---- | ------ |
| 1 | 47 | 0 | 0% |
| 2 | 35 | 0 | 0% |
| 3 | 13 | 0 | 0% |
| 4 | 20 | 0 | 0% |
| 5 | 22 | 0 | 0% |
| **合計** | **137** | **0** | **0%** |
