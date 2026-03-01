# 実装計画: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c`
**日付**: 2026-02-27
**仕様書**: [spec.md](./spec.md)
**データモデル**: [data-model.md](./data-model.md)

## 概要

3層エージェントアーキテクチャ（Lead / Coordinator / Worker）によるプロジェクト統括機能を実装する。

- **Lead**（gwt内蔵AI）がプロジェクト全体を管理し、対話モード（ユーザー質問→コードベース調査→回答）と計画モード（収集→整理→仕様化→委譲）を使い分ける
- **Coordinator** が Issue単位（1 Issue = 1 Coordinator）でタスク管理・CI監視・Worker管理を行う（複数並列起動可、cwd=リポジトリルート）
- **Worker** がペルソナに基づく専門性を持ち、各Worktreeで実装を実行する（1 Task = N Worker = N Worktree）

## 技術コンテキスト

- 言語: Rust 2021 Edition / TypeScript
- GUI: Tauri v2 + Svelte 5 + xterm.js
- バックエンド: gwt-tauri + gwt-core
- ストレージ: ファイルシステム（`~/.gwt/sessions/`）
- テスト: cargo test / pnpm test / vitest
- CI/CD: GitHub Actions
- 既存資産: SessionStore（gwt-core、ファイル永続化実装済み・AppState未接続）、ProjectModePanel（Svelte 5で拡張対象）

## 実装スコープ

### Phase A: 基盤モデル・型定義

1. エンティティモデル定義（ProjectModeSession / ProjectIssue / ProjectTask / CoordinatorState / WorkerState）
2. フロント型定義の3層対応（ProjectModeState / LeadState / DashboardIssue / DashboardTask / CoordinatorState / WorkerState）
3. PTY通信のスキル化基盤（agent_tools.rs → Codex/Geminiローカルskills + Claude Codeプラグイン）
4. SessionStore の AppState ワイヤリング（既存 gwt-core SessionStore を ProjectModeSession 対応に拡張）
5. Skill/Plugin登録スコープ設定モデル（default_scope + Agent別上書き）
6. WorkerPersonaモデル定義（WorkerPersona / PersonaStore / PersonaScope）`crates/gwt-core/src/agent/persona.rs`

### Phase B: Lead（PM）機能

1. Lead実行基盤（gwt内蔵AIとして実装、既存 agent_master.rs の拡張）
2. Leadチャット（ユーザー対話、IME/スピナー/自動スクロール）
3. GitHub Issue仕様管理基盤（issue_specツール群連携、スキル公開）
4. 仕様管理ワークフロー実行（clarify → GitHub Issueにspec/plan/tasks/tdd記録）
5. 仕様4セクションゲート（GitHub Issueにspec/plan/tasks/tdd揃うまでCoordinator起動不可）
6. 計画提示・承認フロー
7. GitHub Issue作成・GitHub Project登録
8. 段階的委譲（自律範囲の判定ロジック）
9. ハイブリッド常駐（イベント駆動 + 2分間隔ポーリング）
10. 途中経過報告（scrollback読み取り + チャット報告、LLMコール不要）
11. Coordinator→Lead ハイブリッド通信（重要イベント: Tauriイベント、途中経過: scrollback読み取り）
12. Lead用ツール拡張（read_file / list_directory / search_code / get_git_status / list_personas）
13. Lead対話モード（ユーザー質問→コードベース調査→回答）
14. Lead計画モード（収集→整理→仕様化→委譲フェーズ管理）
15. プロジェクト知識構築（セッション開始時リポジトリスキャン）
16. システムプロンプト動的生成（静的部分 + リポジトリ情報 + ペルソナ一覧）

### Phase C: Coordinator（Orchestrator）機能

1. Coordinator起動・管理（GUI内蔵ターミナルペイン、cwd=リポジトリルート）
2. Issue単位のCoordinator起動（1 Issue = 1 Coordinator、複数並列可）
3. タスク分割・Worker割り当て（1 Task = N Worker = N Worktree対応）
4. Worktree/ブランチ自動作成（`agent/`プレフィックス、命名規則）
5. Worker起動プロンプト生成（アダプティブ、CLAUDE.md規約含む、エージェント種別に応じた自動モードフラグ）
6. Worker完了検出（Hook Stop / GWT_TASK_DONE / プロセス終了）
7. 並列実行制御（同時実行数のLLM判断）
8. CI監視と自律修正ループ（CI失敗 → Worker修正 → 再プッシュ、最大3回）
9. 成果物検証（テスト実行 → PR作成）
10. Worker間コンテキスト共有（Git merge経由）
11. Coordinatorのペルソナ選定ロジック（タスクtags分析→ペルソナマッチ→Worker起動）

### Phase D: GUI（プロジェクトモードUI）

1. Project Modeタブ・モード切り替え
2. ダッシュボード（左カラム：Issue/Task/Worker階層表示、ステータスバッジ、折りたたみ/展開）
3. Leadチャット画面（右カラム：バブル表示、入力エリア、進捗インライン表示）
4. ダッシュボード内Coordinator詳細展開（ステータス、CI結果、Worker一覧、ターミナル/チャットリンク）
5. Worker表示のBranch Mode連携（TaskクリックでWorktreeジャンプ）
6. コスト可視化（APIコール数/推定トークン数）
7. AI設定未構成時のエラー表示
8. ブランチモードGitHub Issueボタン（gwt内蔵AIがissue_specツールでGitHub Issue管理を実行）
9. ペルソナ設定画面（Settings > Worker Personas: CRUD、タグ、スコープ表示）
10. チャットUI強化（Markdownレンダリング、折りたたみ表示、承認ボタン、タイピングインジケーター、フェーズ表示）

### Phase E: セッション・障害・連携

1. セッション永続化（ProjectModeSession → JSON、~/.gwt/sessions/）
2. セッション復元・再開（Coordinator/Worker再接続）
3. 層間独立性（Lead障害 / Coordinator障害 / Worker障害の各ハンドリング）
4. Coordinator自律再起動（Lead検出 → 再起動 → Worker状態再取得）
5. セッション強制中断（Esc → SIGTERM → Paused永続化）
6. ブランチモード連携（agent/ブランチ表示・削除検出）
7. コンテキスト要約・圧縮（Worker: LLM自動、Lead/Coordinator: gwt側80%閾値制御）

### Phase F: スキル化・統合

1. PTY通信の統合スキル化（Codex/Gemini: `~/.{codex,gemini}/skills`, Claude Code: gwtプラグイン）
2. issue_specツールの統合スキル化（ブランチモード各エージェントからも利用可能に）
3. agent_tools.rs PTYツールからの完全移行
4. Claude Code Hook転送のプラグイン同梱化（`hooks.json` + `forward-gwt-hook.sh`）
5. GUI起動時の手動Hook登録ダイアログ廃止（plugin setup自動登録へ統一）
6. Skill/Plugin登録先のScope解決（User/Project/Local + Agent別上書き）
7. 直接アクセス（Workerターミナル直操作 / Coordinatorチャット）

### Phase G: 仕上げ

1. ログ記録実装（agent.lead.llm / agent.coordinator / agent.worker）
2. 仕様・計画・タスクの最終同期確認

## 主要コード構成

```text
crates/gwt-core/src/
├── agent/
│   ├── mod.rs                    # 3層エージェントモデル（ProjectModeSession）
│   ├── lead.rs                   # LeadState / LeadStatus / LeadMessage
│   ├── coordinator.rs            # CoordinatorState / CoordinatorStatus
│   ├── worker.rs                 # WorkerState / WorkerStatus
│   ├── persona.rs                # WorkerPersona / PersonaStore / PersonaScope
│   ├── issue.rs                  # ProjectIssue / IssueStatus
│   ├── task.rs                   # ProjectTask（拡張: workers Vec）
│   ├── session.rs                # ProjectModeSession（旧AgentSession拡張）
│   ├── session_store.rs          # SessionStore（ProjectModeSession対応）
│   ├── worktree.rs               # WorktreeRef（既存資産）
│   ├── conversation.rs           # LeadMessage（kind追加）
│   ├── scanner.rs                # RepositoryScanner（既存資産）
│   └── prompt_builder.rs         # PromptBuilder（既存資産、拡張）

crates/gwt-tauri/src/
├── agent_master.rs               # Lead実行ループ（GitHub Issue仕様管理統合、ハイブリッド常駐）
├── agent_tools.rs                # LLMツール定義（PTY通信3ツール + issue_spec 7ツール）
├── state.rs                      # AppState（SessionStoreワイヤリング追加）
├── commands/
│   ├── project_mode.rs           # Project Mode用Tauriコマンド（拡張）
│   └── terminal.rs               # PTY通信（既存、スキル化元）
└── context_summarizer.rs         # Lead/Coordinator用コンテキスト要約

gwt-gui/src/
├── lib/components/
│   ├── ProjectModePanel.svelte   # メインパネル（2カラム）
│   ├── LeadChat.svelte           # Leadチャット（右カラム）
│   ├── Dashboard.svelte          # ダッシュボード（左カラム）
│   ├── IssueItem.svelte          # Issue階層表示コンポーネント
│   ├── CoordinatorDetail.svelte  # Coordinator詳細展開
│   ├── PersonaSettings.svelte    # ペルソナ設定画面
│   └── MarkdownRenderer.svelte   # Markdown表示（既存再利用）
└── lib/types.ts                  # 3層対応型定義
```

## 実装方針

- 既存のProjectModePanel/AgentSidebarを拡張して3層対応する（Svelte 5）
- Lead実行ループは既存のagent_master.rsを拡張する（ReActループ → GitHub Issue仕様管理統合ループ）
- gwt-core の既存 SessionStore を ProjectModeSession 対応に拡張し、AppState にワイヤリングする
- Coordinator/Workerは新規の状態管理モジュールとして追加する
- GUI UIはダッシュボード（左）+ Leadチャット（右）の2カラム構成で実装する
- Coordinator→Lead通信はハイブリッド（重要イベント: Tauriイベント、途中経過: scrollback読み取り）
- Workerのエージェント種別は、起動時に固定指定がある場合はそれを優先し、未指定時は`persona.agent_type`、未設定なら`claude`を使用する
- コンテキスト要約はWorkerはLLM自動、Lead/Coordinatorはgwt側で80%閾値制御
- PTY通信は既存のsend_keys系を維持しつつ、スキルとしてのインターフェースを追加する
- Claude Code Hook連携は`plugins/gwt-integration/hooks/hooks.json`を正本とし、5イベントを`gwt-tauri hook <Event>`へ転送する
- 起動時は`repair_skill_registration`でClaudeプラグイン登録をベストエフォート自動修復し、手動Hook登録ダイアログには依存しない
- Skill/Plugin登録は`default_scope`（User/Project/Local）を基準にし、Agent別上書きがある場合は上書きを優先する
- Scope別の登録先はCodex/Geminiはskillsディレクトリ、Claudeはsettingsファイル（User/Project/Local）で解決する
- テストは各フェーズごとにTDD（テストファースト）で進める

## リスクと緩和策

| リスク | 影響度 | 緩和策 |
|---|---|---|
| LLMコンテキスト枯渇（Lead長時間運用） | 高 | gwt側80%閾値での要約圧縮 |
| SessionStore未接続による永続化ギャップ | 高 | Phase A でAppStateへのワイヤリングを最優先 |
| Coordinator並列数増加によるリソース逼迫 | 中 | Coordinator/Worker数の上限をLLM判断で制御 |
| Claude Code Agent Team APIの変更 | 中 | Coordinator実行基盤を抽象化し差し替え可能に |
| ProjectModePanel 拡張時の回帰 | 低 | 既存テストを維持しつつ段階的に移行 |

## 依存関係

- 既存のAI要約機能（SPEC-4b893dae）のAPI設定を共有
- 既存のGUIターミナルベースのエージェント起動機能（terminal.rs）
- 既存のworktree管理機能（gwt-core）
- Claude Code Hook機能（Stop）
- Claude Code Agent Team機能（Coordinator実行基盤）
- `gh` CLI（GitHub Issue/PR操作）

## 受け入れ条件

- Leadチャットでユーザーと対話し、プロジェクト全体の要件定義・GitHub Issueへの仕様記録を実行できる
- Leadが要件をIssue単位に分割し、各GitHub Issueに仕様4セクション（spec/plan/tasks/tdd）を記録できる
- 承認後、各IssueにCoordinatorを並列起動できる（GitHub Issue番号受け渡し）
- 1 Task = N Worker = N Worktreeの並列割り当てが可能
- ダッシュボードでIssue/Task/Workerの階層をステータス付きで常時俯瞰できる
- ダッシュボードのTaskクリックでBranch Modeの該当Worktreeにジャンプできる
- CoordinatorがCI結果を監視し、失敗時に自律修正ループを実行できる
- 各層が独立して動作し、上位層の障害が下位層に影響しない
- セッションを永続化し、gwt再起動後に復元・再開できる
- IME/スピナー/自動スクロールの既存チャット要件を維持する
- PTY通信がCodex/Claude Code/Geminiスキルとして利用可能
- Claude Code Hook転送がプラグイン経由で自動有効化され、手動Hookセットアップなしで完了検出が動作する
- SettingsでScope（User/Project/Local）とAgent別上書きを設定し、repair/statusが設定どおりの登録先に対して動作する
- ペルソナ設定画面でペルソナのCRUD操作が可能
- CoordinatorがタスクのtagsからペルソナをマッチングしてWorkerを起動できる
- Leadがread_file/search_codeで自律的にリポジトリを調査し、ユーザーの質問に回答できる
- Leadが収集→整理→仕様化→委譲の計画モードフローを実行できる
- チャットUIでMarkdownレンダリングと折りたたみ表示が動作する

## 検証方針

- フロントエンド: ダッシュボード、チャット、Branch Mode連携をコンポーネントテスト（vitest）で検証
- バックエンド: Lead/Coordinator/Worker状態管理、仕様4セクションゲート、CI監視をユニットテスト（cargo test）で検証
- 回帰: 既存ProjectModePanel/AgentSidebarの機能テストを維持
- 統合: Coordinator→Worker起動→完了検出→PR作成のE2Eフローを検証
