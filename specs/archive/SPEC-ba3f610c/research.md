# フェーズ0: 調査結果

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-07

## 1. 既存コードベース分析

### 1.1 gwt-core: agent/ モジュール

| ファイル | 役割 | 現在の状態 | 拡張ポイント |
|---------|------|-----------|-------------|
| `mod.rs` | AgentManager、AgentType、detect/launch API | 実装済み | SubAgentType連携、自動モード起動フラグ追加 |
| `master.rs` | MasterAgent（AIClient + Conversation） | 基本実装済み | イベント駆動ループ、PromptBuilder統合、Spec Kit連携 |
| `session.rs` | AgentSession（状態・タスク・WT参照） | 基本実装済み | 永続化I/O、復元ロジック、キュー管理、base_branch等 |
| `task.rs` | Task（状態遷移・依存関係・WT割当） | 基本実装済み | テスト検証状態、PR参照、リトライカウンタ |
| `sub_agent.rs` | SubAgent（tmuxペイン・状態・完了検出） | 基本実装済み | 全自動モードフラグ、send-keys完了確認 |
| `conversation.rs` | Conversation + Message（対話履歴） | 実装済み | 要約圧縮（P3） |
| `types.rs` | SessionId / TaskId / SubAgentId（UUID v4） | 実装済み | 変更不要 |
| `worktree.rs` | WorktreeRef（ブランチ名・パス・タスクID参照） | 実装済み | クリーンアップ用メソッド追加 |
| `trait_agent.rs` | AgentTrait（detect/run_task/run_in_directory） | 実装済み | 全自動モード判定追加 |
| `claude.rs` / `codex.rs` / `gemini.rs` | 各エージェント実装 | 実装済み | 全自動モード起動引数 |

### 1.2 gwt-core: tmux/ モジュール

| ファイル | 役割 | 現在の状態 | 拡張ポイント |
|---------|------|-----------|-------------|
| `launcher.rs` | TmuxLaunchConfig + launch_agent_in_pane | 実装済み | 全自動モードフラグ対応（env/args拡張） |
| `pane.rs` | AgentPane + ペイン操作（kill/send-keys/capture-pane） | 実装済み | send-keys完了確認、capture-paneパターン検出 |
| `poller.rs` | PanePoller（mpsc::channel、バックグラウンドポーリング） | 実装済み | イベント駆動通知モード追加 |
| `detector.rs` | tmux環境検出（is_inside_tmux等） | 実装済み | 変更不要 |
| `session.rs` | tmuxセッション管理 | 実装済み | 変更不要 |

### 1.3 gwt-core: ai/ モジュール

| ファイル | 役割 | 現在の状態 | 拡張ポイント |
|---------|------|-----------|-------------|
| `client.rs` | AIClient（OpenAI互換API、Responses API） | 実装済み | コスト追跡（トークン数推定）、MAX_OUTPUT_TOKENS拡張 |

### 1.4 gwt-cli: TUI

| ファイル | 役割 | 現在の状態 | 拡張ポイント |
|---------|------|-----------|-------------|
| `agent_mode.rs` | AgentModeState + render（チャット70/タスク30分割） | 実装済み | チャットのみUI刷新、タスクパネル削除、ステータスバー追加 |
| `app.rs` | メインアプリループ、画面切り替え | 実装済み | Esc中断、キュー管理、Spec Kitショートカット |

### 1.5 既存Spec Kitスクリプト（.specify/）

`.specify/`ディレクトリには以下が存在:

- `.specify/memory/constitution.md` - プロジェクト原則
- `.specify/scripts/bash/*.sh` - セットアップ・コンテキスト更新スクリプト
- Spec Kit Skillsは`projectSettings`のスキル定義として存在（speckit.clarify, speckit.plan, speckit.tasks等）

これらのプロンプト構造をLLMテンプレートとしてRustバイナリに内蔵する。

## 2. 技術的決定

### 2.1 イベント駆動ループ

- **選定**: `std::sync::mpsc::channel`（既存のPanePollerパターンを踏襲）
- **理由**: 既にPollerがmpscで実装されており、パターンの一貫性を維持できる。外部ランタイム（tokio等）不要。
- **イベント型**: `OrchestratorEvent` enum（SubAgentCompleted / SubAgentFailed / UserInput / TimerTick / SessionStart）

### 2.2 セッション永続化

- **選定**: serde_json + atomic file write
- **保存先**: `~/.gwt/sessions/{session_id}.json`
- **書込方式**: 一時ファイル（`.tmp`接尾辞）→ atomic rename。既存のログファイルパターン流用。
- **パーミッション**: ファイル0600、ディレクトリ0700

### 2.3 Spec Kit LLMプロンプト

- **選定**: `include_str!`マクロでコンパイル時埋め込み
- **理由**: バイナリ配布のため外部ファイル依存を排除。テンプレート更新はリリースに同期。
- **テンプレート形式**: マークダウン形式のプロンプト（変数プレースホルダー`{{variable}}`使用）

### 2.4 リポジトリディープスキャン

- **実装**: `git ls-tree -r --name-only HEAD` + 選択的ファイル読み取り
- **スキャン対象**:
  - `CLAUDE.md` / `.claude/` → プロジェクト規約
  - `Cargo.toml` / `package.json` → 依存関係・ビルドシステム
  - `src/` or `crates/` のモジュール構成（ディレクトリツリー）
  - `specs/` → 既存スペック一覧
- **キャッシュ**: セッション単位でメモリキャッシュ（1回スキャン）

### 2.5 完了検出方式

| エージェント | 主方式 | フォールバック |
|-------------|-------|-------------|
| Claude Code | Hook Stop（ファイルシグナル） | tmux複合方式 |
| Codex | プロセス終了監視 | 出力パターン (`GWT_TASK_DONE`) |
| Gemini | プロセス終了監視 | 出力パターン (`GWT_TASK_DONE`) |
| Other | 出力パターン | send-keys確認クエリ |

### 2.6 全自動モード起動フラグ

| エージェント | フラグ |
|-------------|------|
| Claude Code | `--dangerously-skip-permissions` |
| Codex | `--full-auto` |
| Gemini | `--sandbox`（利用可能な場合） |

## 3. 制約と依存関係

### 3.1 ハード依存

- **tmux 3.0+**: `send-keys`, `capture-pane`, `list-panes`, `split-window` コマンドが必要
- **OpenAI互換API**: 既存AIClientを共有。`Responses API`形式で通信
- **gh CLI**: PR作成に必要（未インストール時はPR作成をスキップ、ユーザーに通知）

### 3.2 ソフト依存

- Claude Code: Hook機能（Stop）がHook APIの変更リスクあり → tmux複合方式をフォールバック
- サブエージェント: 少なくとも1つのコーディングエージェント（Claude Code / Codex / Gemini）がインストール済み

### 3.3 パフォーマンス制約

- モード切り替え: 1秒以内（TUI状態遷移のみ）
- LLM初回応答: 5秒以内（API応答時間依存）
- 完了検出: 10秒以内（ポーリング間隔1秒）
- セッション永続化: 状態変更から1秒以内

## 4. 未解決事項

なし（インタビュー14ラウンドですべて解消済み）
