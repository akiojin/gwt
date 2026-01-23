# タスク: エージェントモード

**入力**: `/specs/SPEC-ba3f610c/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）、research.md、data-model.md、quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## ストーリー依存関係

```text
US1 (モード切り替え) ──┐
                      ├──► US5 (成果物統合)
US2 (タスク分割) ─────┤
                      ├──► US6 (失敗ハンドリング)
US3 (サブエージェント起動) ──┤
                      ├──► US7 (セッション永続化)
US4 (完了検出) ───────┘
                              │
                              ▼
                        US8 (コンテキスト管理)
```

- **P1グループ** (US1-4): 並列開発可能だが、統合テストには全て必要
- **P2グループ** (US5-7): P1完了後に並列開発可能
- **P3グループ** (US8): P2完了後

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: agentモジュールの基盤構造を構築

### セットアップタスク

- [ ] **T001** [P] [共通] `crates/gwt-core/src/agent/mod.rs` にagentモジュールを作成し、サブモジュールをエクスポート
- [ ] **T002** [P] [共通] `crates/gwt-core/src/lib.rs` に `pub mod agent;` を追加
- [ ] **T003** [P] [共通] `crates/gwt-core/src/agent/types.rs` に共通型定義（SessionId, TaskId, SubAgentId）を作成

## フェーズ2: 基盤（データモデル）

**目的**: data-model.md に基づくRust構造体の実装

### データ層

- [ ] **T004** [P] [共通] `crates/gwt-core/src/agent/session.rs` にAgentSession, SessionStatus構造体を実装
- [ ] **T005** [P] [共通] `crates/gwt-core/src/agent/task.rs` にTask, TaskStatus, TaskResult, WorktreeStrategy構造体を実装
- [ ] **T006** [P] [共通] `crates/gwt-core/src/agent/conversation.rs` にConversation, Message, MessageRole構造体を実装
- [ ] **T007** [P] [共通] `crates/gwt-core/src/agent/sub_agent.rs` にSubAgent, SubAgentType, SubAgentStatus, CompletionSource構造体を実装
- [ ] **T008** [P] [共通] `crates/gwt-core/src/agent/worktree.rs` にWorktreeRef構造体を実装
- [ ] **T009** [共通] T004-T008の後に `crates/gwt-core/src/agent/mod.rs` を更新して全構造体をpub useでエクスポート

## フェーズ3: ユーザーストーリー1 - モード切り替えと基本対話 (優先度: P1)

**ストーリー**: 開発者がブランチモードで`Tab`を押すと、エージェントモードに切り替わり、マスターエージェントとの対話画面が表示される

**価値**: エージェントモードへの入口を提供し、マスターエージェントとの対話基盤を確立

### ブランチモードUI変更（FR-001a）

- [ ] **T101** [US1] `crates/gwt-cli/src/tui/screens/branch_mode.rs` のレイアウトを変更し、詳細パネルとAI要約パネルを縦並びに
- [ ] **T102** [US1] T101の後に `crates/gwt-cli/src/tui/screens/branch_mode.rs` から既存Tab切り替えロジックを削除

### エージェントモード画面

- [ ] **T103** [P] [US1] `crates/gwt-cli/src/tui/screens/agent_mode.rs` を新規作成し、AgentModeState構造体を定義
- [ ] **T104** [US1] T103の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にrender_agent_mode関数を実装（チャット画面UI）
- [ ] **T105** [US1] T104の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にユーザー入力フィールドとメッセージ履歴表示を実装
- [ ] **T106** [US1] `crates/gwt-cli/src/tui/screens/mod.rs` にagent_modeモジュールを追加しエクスポート

### モード切り替えロジック

- [ ] **T107** [US1] T106の後に `crates/gwt-cli/src/tui/app.rs` にAppMode列挙型を追加（BranchMode, AgentMode）
- [ ] **T108** [US1] T107の後に `crates/gwt-cli/src/tui/app.rs` のキーハンドラにTabキーでモード切り替え処理を追加
- [ ] **T109** [US1] T108の後に `crates/gwt-cli/src/tui/app.rs` のview関数でAppModeに応じた画面描画を実装

### マスターエージェント基盤

- [ ] **T110** [P] [US1] `crates/gwt-core/src/agent/master.rs` にMasterAgent構造体を作成（AIClientを内包）
- [ ] **T111** [US1] T110の後に `crates/gwt-core/src/agent/master.rs` にsend_message関数を実装（ユーザーメッセージをLLMに送信）
- [ ] **T112** [US1] T111の後に `crates/gwt-core/src/agent/master.rs` にシステムプロンプトを定義（タスク分析・計画策定用）

### AI設定チェック

- [ ] **T113** [US1] T109の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にAI設定未構成時のエラーメッセージ表示を実装

**✅ MVP1チェックポイント**: US1完了後、モード切り替えとマスターエージェントとの基本対話が可能

## フェーズ4: ユーザーストーリー2 - タスク分割とWorktree自動作成 (優先度: P1)

**ストーリー**: マスターエージェントがユーザーのタスクを分析し、サブタスクに分割してworktreeを自動作成する

**価値**: 自律的なタスク分割とWorktree管理でマルチタスク並列実行の基盤を提供

### タスク管理

- [ ] **T201** [US2] `crates/gwt-core/src/agent/task.rs` にTaskManager構造体を追加（タスク一覧管理）
- [ ] **T202** [US2] T201の後に `crates/gwt-core/src/agent/task.rs` にadd_task, get_ready_tasks, update_status関数を実装
- [ ] **T203** [US2] T202の後に `crates/gwt-core/src/agent/task.rs` に依存関係解決ロジック（check_dependencies）を実装

### タスク分割プロンプト

- [ ] **T204** [US2] T112の後に `crates/gwt-core/src/agent/master.rs` にanalyze_task関数を追加（LLMにタスク分割を依頼）
- [ ] **T205** [US2] T204の後に `crates/gwt-core/src/agent/master.rs` にタスク分割結果のJSONパース処理を実装
- [ ] **T206** [US2] T205の後に `crates/gwt-core/src/agent/master.rs` にWorktree戦略判断プロンプトを追加

### Worktree自動作成

- [ ] **T207** [P] [US2] `crates/gwt-core/src/agent/worktree.rs` にWorkreeManager構造体を追加
- [ ] **T208** [US2] T207の後に `crates/gwt-core/src/agent/worktree.rs` にcreate_worktree関数を実装（agent/プレフィックス付きブランチ作成）
- [ ] **T209** [US2] T208の後に `crates/gwt-core/src/agent/worktree.rs` にブランチ名生成ロジック（sanitize_branch_name）を実装

### TUI連携

- [ ] **T210** [US2] T206の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にタスク一覧表示パネルを追加
- [ ] **T211** [US2] T210の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にタスク実行承認UIを追加

**✅ MVP2チェックポイント**: US2完了後、タスク分割とworktree自動作成が可能

## フェーズ5: ユーザーストーリー3 - サブエージェント起動と指示 (優先度: P1)

**ストーリー**: マスターエージェントがサブエージェントをtmuxペインで起動し、プロンプトを送信する

**価値**: サブエージェントへの作業委譲でタスク実行の実体を実現

### オーケストレーター

- [ ] **T301** [P] [US3] `crates/gwt-core/src/agent/orchestrator.rs` にOrchestrator構造体を作成
- [ ] **T302** [US3] T301の後に `crates/gwt-core/src/agent/orchestrator.rs` にstart_sub_agent関数を実装（tmuxペイン作成）
- [ ] **T303** [US3] T302の後に `crates/gwt-core/src/agent/orchestrator.rs` にsend_prompt関数を実装（tmux send-keys）

### プロンプト生成

- [ ] **T304** [US3] T303の後に `crates/gwt-core/src/agent/orchestrator.rs` にbuild_prompt関数を実装（タスク指示+終了指示を含む）
- [ ] **T305** [US3] T304の後に `crates/gwt-core/src/agent/orchestrator.rs` に「完了したらqで終了」の指示テンプレートを追加

### tmux制御拡張

- [ ] **T306** [US3] `crates/gwt-core/src/tmux/pane.rs` にcreate_pane_in_worktree関数を追加（worktree内でペイン作成）
- [ ] **T307** [US3] T306の後に `crates/gwt-core/src/tmux/launcher.rs` にlaunch_agent関数を追加（エージェントコマンド起動）

### 並列実行

- [ ] **T308** [US3] T307の後に `crates/gwt-core/src/agent/orchestrator.rs` にrun_parallel_tasks関数を実装（複数サブエージェント並列起動）

**✅ MVP3チェックポイント**: US3完了後、サブエージェントの起動と指示送信が可能

## フェーズ6: ユーザーストーリー4 - サブエージェント完了検出 (優先度: P1)

**ストーリー**: マスターエージェントがサブエージェントの完了を検出する

**価値**: タスク進行管理と次ステップへの移行を実現

### Claude Code Hook検出

- [ ] **T401** [P] [US4] `crates/gwt-core/src/agent/detector.rs` を新規作成し、CompletionDetector traitを定義
- [ ] **T402** [US4] T401の後に `crates/gwt-core/src/agent/detector.rs` にHookDetector構造体を実装（Claude Code Hook Stop経由）
- [ ] **T403** [US4] T402の後に `crates/gwt-core/src/tmux/detector.rs` にHook完了イベント監視ロジックを追加

### tmux複合方式検出

- [ ] **T404** [US4] T401の後に `crates/gwt-core/src/agent/detector.rs` にTmuxCompositeDetector構造体を実装
- [ ] **T405** [US4] T404の後に `crates/gwt-core/src/agent/detector.rs` にprocess_exit_check関数を実装（pane_dead検出）
- [ ] **T406** [US4] T405の後に `crates/gwt-core/src/agent/detector.rs` にoutput_pattern_check関数を実装（capture-pane + パターンマッチ）
- [ ] **T407** [US4] T406の後に `crates/gwt-core/src/agent/detector.rs` にactivity_check関数を実装（アイドルタイムアウト検出）

### 検出統合

- [ ] **T408** [US4] T407の後に `crates/gwt-core/src/agent/detector.rs` にdetect_completion関数を実装（複合判定）
- [ ] **T409** [US4] T408の後に `crates/gwt-core/src/agent/orchestrator.rs` に完了検出ループを追加（poll_completions）

**✅ P1完了チェックポイント**: US1-4完了後、基本的なエージェントモードフローが動作

## フェーズ7: ユーザーストーリー5 - 成果物統合（PR経由） (優先度: P2)

**ストーリー**: 複数タスク完了後、各worktreeからPRを作成して成果物を統合する

**価値**: 分散作業の成果物を統合し、コードレビューフローに乗せる

### PR作成

- [ ] **T501** [P] [US5] `crates/gwt-core/src/agent/integrator.rs` を新規作成し、Integrator構造体を定義
- [ ] **T502** [US5] T501の後に `crates/gwt-core/src/agent/integrator.rs` にcreate_pr関数を実装（gh pr create呼び出し）
- [ ] **T503** [US5] T502の後に `crates/gwt-core/src/agent/integrator.rs` にgenerate_pr_body関数を実装（タスク結果からPR説明生成）

### コンフリクト検出

- [ ] **T504** [US5] T503の後に `crates/gwt-core/src/agent/integrator.rs` にcheck_merge_conflicts関数を実装
- [ ] **T505** [US5] T504の後に `crates/gwt-core/src/agent/integrator.rs` にrequest_conflict_resolution関数を実装（サブエージェントに解決指示）

### マスターエージェント連携

- [ ] **T506** [US5] T505の後に `crates/gwt-core/src/agent/master.rs` にintegrate_results関数を追加（統合フェーズ開始）

**✅ MVP4チェックポイント**: US5完了後、PR経由の成果物統合が可能

## フェーズ8: ユーザーストーリー6 - 失敗ハンドリング (優先度: P2)

**ストーリー**: サブエージェントがタスクに失敗した場合、マスターエージェントがLLM判断で対応する

**価値**: 自律的なエラー回復で人間の介入を最小化

### 失敗検出

- [ ] **T601** [P] [US6] `crates/gwt-core/src/agent/error_handler.rs` を新規作成し、ErrorHandler構造体を定義
- [ ] **T602** [US6] T601の後に `crates/gwt-core/src/agent/error_handler.rs` にdetect_failure関数を実装（エラー終了検出）
- [ ] **T603** [US6] T602の後に `crates/gwt-core/src/agent/error_handler.rs` にanalyze_failure関数を実装（出力からエラー内容を抽出）

### 対応策決定

- [ ] **T604** [US6] T603の後に `crates/gwt-core/src/agent/master.rs` にhandle_failure関数を追加（LLM判断で対応策決定）
- [ ] **T605** [US6] T604の後に `crates/gwt-core/src/agent/master.rs` に失敗対応プロンプトを追加（リトライ/代替/相談の選択）

### リトライ・代替実行

- [ ] **T606** [US6] T605の後に `crates/gwt-core/src/agent/orchestrator.rs` にretry_task関数を追加
- [ ] **T607** [US6] T606の後に `crates/gwt-core/src/agent/orchestrator.rs` にexecute_alternative関数を追加

**✅ MVP5チェックポイント**: US6完了後、失敗時の自律回復が可能

## フェーズ9: ユーザーストーリー7 - セッション永続化と再開 (優先度: P2)

**ストーリー**: エージェントモードのセッション状態を完全永続化し、再起動後も再開可能にする

**価値**: 長時間タスクの中断・再開をサポート

### 永続化

- [ ] **T701** [P] [US7] `crates/gwt-core/src/agent/persistence.rs` を新規作成し、SessionPersistence構造体を定義
- [ ] **T702** [US7] T701の後に `crates/gwt-core/src/agent/persistence.rs` にsave_session関数を実装（~/.gwt/sessions/へJSON保存）
- [ ] **T703** [US7] T702の後に `crates/gwt-core/src/agent/persistence.rs` にauto_save関数を実装（状態変更時の自動保存）

### 復元

- [ ] **T704** [US7] T703の後に `crates/gwt-core/src/agent/persistence.rs` にload_session関数を実装（JSON読み込み）
- [ ] **T705** [US7] T704の後に `crates/gwt-core/src/agent/persistence.rs` にlist_sessions関数を実装（セッション一覧取得）
- [ ] **T706** [US7] T705の後に `crates/gwt-core/src/agent/persistence.rs` にvalidate_session関数を実装（worktree存在確認等）

### TUI連携

- [ ] **T707** [US7] T706の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にセッション選択UIを追加
- [ ] **T708** [US7] T707の後に `crates/gwt-cli/src/tui/app.rs` にセッション復元処理を追加（起動時）

**✅ MVP6チェックポイント**: US7完了後、セッション永続化と再開が可能

## フェーズ10: ユーザーストーリー8 - コンテキスト管理（要約圧縮） (優先度: P3)

**ストーリー**: 大規模タスクでコンテキストウィンドウを超える場合、完了タスクの情報を要約圧縮する

**価値**: 大規模・長時間タスクへの対応を実現

### コンテキスト監視

- [ ] **T801** [P] [US8] `crates/gwt-core/src/agent/context.rs` を新規作成し、ContextManager構造体を定義
- [ ] **T802** [US8] T801の後に `crates/gwt-core/src/agent/context.rs` にestimate_tokens関数を実装（トークン数推定）
- [ ] **T803** [US8] T802の後に `crates/gwt-core/src/agent/context.rs` にshould_compress関数を実装（圧縮必要判定）

### 要約圧縮

- [ ] **T804** [US8] T803の後に `crates/gwt-core/src/agent/context.rs` にcompress_context関数を実装（LLM要約呼び出し）
- [ ] **T805** [US8] T804の後に `crates/gwt-core/src/agent/context.rs` に要約プロンプトを定義（重要情報保持）

### マスターエージェント統合

- [ ] **T806** [US8] T805の後に `crates/gwt-core/src/agent/master.rs` にコンテキスト管理を統合（送信前に圧縮チェック）

**✅ 完全な機能**: US8完了後、すべての要件が満たされます

## フェーズ11: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### 統合

- [ ] **T901** [統合] エンドツーエンドの統合テスト実行（エージェントモード起動→タスク入力→完了まで）
- [ ] **T902** [統合] エッジケース対応（tmux切断復旧、worktree削除時の対処）
- [ ] **T903** [統合] `cargo clippy --all-targets --all-features -- -D warnings` をローカルで完走させ、失敗時は修正
- [ ] **T904** [統合] `cargo fmt` を実行しフォーマット統一
- [ ] **T905** [統合] `cargo test` を実行し全テストパス確認

### ドキュメント

- [ ] **T906** [P] [ドキュメント] `README.ja.md` にエージェントモードの使い方を追記
- [ ] **T907** [P] [ドキュメント] `specs/SPEC-ba3f610c/quickstart.md` を最新実装に合わせて更新

### 最終確認

- [ ] **T908** [統合] SC-001〜SC-005の成功基準を検証
- [ ] **T909** [統合] `bunx markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` を完走させ、失敗時は修正

## タスク凡例

**優先度**:

- **P1**: 最も重要 - 基本フローに必要（US1-4）
- **P2**: 重要 - 完全な機能に必要（US5-7）
- **P3**: 補完的 - 大規模タスク対応（US8）

**依存関係**:

- **[P]**: 並列実行可能
- **依存あり**: 前のタスク完了後に実行

**ストーリータグ**:

- **[US1]**: モード切り替えと基本対話
- **[US2]**: タスク分割とWorktree自動作成
- **[US3]**: サブエージェント起動と指示
- **[US4]**: サブエージェント完了検出
- **[US5]**: 成果物統合（PR経由）
- **[US6]**: 失敗ハンドリング
- **[US7]**: セッション永続化と再開
- **[US8]**: コンテキスト管理（要約圧縮）
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 注記

- 各タスクは1時間から1日で完了可能
- 並列実行可能なタスクには `[P]` を付与
- 同一ファイルを触るタスクは直列に配置
- テストタスクはspec.mdで明示的に要求されていないため省略（TDDはCLAUDE.mdで義務付け）
