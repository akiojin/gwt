# 調査メモ: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c` | **調査日**: 2026-02-19

## 既存実装の状況

### gwt-core agent モジュール

既存の `crates/gwt-core/src/agent/` には以下が実装済み:

- **AgentType** enum: Claude / Codex / Gemini（`.command()` で起動コマンド取得）
- **AgentManager**: `detect_agents()` / `get_best_agent()` / `run_task()` / `run_in_worktree()`
- **UUID型**: SessionId / TaskId / SubAgentId（`Uuid::new_v4()` ベース）
- **AgentSession**: id / status / conversation / tasks / worktrees / llm_call_count / estimated_tokens
- **Task**: id / name / status / dependencies / worktree_strategy / sub_agent / test_status / retry_count / pull_request
- **SubAgent**: id / agent_type / pane_id / pid / status / completion_source
- **WorktreeRef**: branch_name / path / task_ids + `sanitize_branch_name()` / `create_agent_branch_name()`
- **Conversation**: Message（role: User/Assistant/System）のベクタ
- **SessionStore**: `~/.gwt/sessions/{session_id}.json` でファイル永続化（アトミック書き込み）
- **RepositoryScanner**: git ls-tree でリポジトリコンテキスト取得
- **PromptBuilder**: Sub-agent用プロンプト構築（CLAUDE.md規約、GWT_TASK_DONEマーカー含む）

### gwt-tauri バックエンド

- **agent_master.rs**: Master Agent ReActループ（最大3回、ブロッキング）、ProjectModeState（インメモリ・ウィンドウ単位）
- **commands/project_mode.rs**: 2コマンドのみ（`get_project_mode_state_cmd` / `send_project_mode_message`）
- **commands/terminal.rs**: PTY管理全般（launch_agent, send_keys_to_pane, send_keys_broadcast, capture_scrollback_tail等）
- **agent_tools.rs**: 10個のLLMツール定義（send_keys系3 + spec issue系7）
- **SessionStore は未接続**: gwt-core に実装済みだが AppState にワイヤリングされていない

### gwt-gui フロントエンド

- **types.ts**: ProjectModeState / LeadMessage / AgentSidebarTask / AgentSidebarSubAgent / Tab等
- **ProjectModePanel.svelte**: Svelte 5。チャットUI + LLMコール数表示
- **AgentSidebar.svelte**: Svelte 5（runes）。5秒ポーリングでタスク/Sub-agent表示

## 再利用可能な資産

| 資産 | 再利用方針 |
|---|---|
| SessionStore（gwt-core） | セッション永続化の基盤としてそのまま拡張 |
| Task / SubAgent / WorktreeRef | 3層モデルの Developer / Worktree としてフィールド追加で拡張 |
| sanitize_branch_name() | agent/ ブランチ命名でそのまま利用 |
| PromptBuilder | Developer起動プロンプト生成の基盤として拡張 |
| RepositoryScanner | Lead の clarify フェーズのリポジトリ分析で利用 |
| send_keys_to_pane / capture_scrollback_tail | PTY通信スキルの実装基盤 |
| ProjectModePanel.svelte | 既存UIを拡張してProject Mode要件へ対応 |

## 新規実装が必要な部分

| 機能 | 理由 |
|---|---|
| Project / Issue エンティティ | 旧モデルに存在しない上位構造 |
| Coordinator 状態管理 | 旧モデルに Orchestrator 層が存在しない |
| Lead 実行ループ（GitHub Issue仕様管理統合） | 旧 ReAct ループを大幅拡張（issue_specツール連携） |
| Coordinator→Lead ハイブリッド通信 | Tauriイベント + scrollback読み取りの二重系 |
| ダッシュボード（Dashboard.svelte） | 旧 AgentSidebar から完全に再設計 |
| LeadChat.svelte | ProjectModePanel からチャット部分を分離・拡張 |
| コンテキスト要約（gwt側制御） | 旧実装にはLead/Coordinator用の要約機能なし |

## 主要リスク

| リスク | 影響度 | 緩和策 |
|---|---|---|
| LLMコンテキスト枯渇（Lead長時間運用） | 高 | gwt側80%閾値での要約圧縮 |
| Coordinator並列数増加によるリソース逼迫 | 中 | Coordinator/Developer数の上限をLLM判断で制御 |
| Claude Code Agent Team APIの変更 | 中 | Coordinator実行基盤を抽象化し差し替え可能に |
| ProjectModePanel 拡張時の回帰 | 低 | 既存テストを維持しつつ段階的に移行 |
| SessionStore未接続による永続化ギャップ | 高 | Phase A でAppStateへのワイヤリングを最優先 |
