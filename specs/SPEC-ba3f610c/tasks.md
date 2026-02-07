# タスク: エージェントモード

**入力**: `/specs/SPEC-ba3f610c/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## ストーリー依存関係

```text
US1 (モード切り替え+対話) ──┐
                           ├──► US5 (成果物検証+PR)
US2 (タスク分割+WT作成) ───┤
                           ├──► US6 (失敗ハンドリング)
US3 (サブエージェント起動) ──┤
                           ├──► US7 (セッション永続化)
US4 (完了検出) ────────────┘
                                  │
                                  ▼
                            US8 (コンテキスト圧縮)
```

- **P1 Phase A** (基盤): Spec Kit内蔵 + チャットUI刷新 + ディープスキャン
- **P1 Phase B** (コア): タスク分割 + WT作成 + サブエージェント起動 + 完了検出
- **P1 Phase C** (E2E): イベント駆動ループ + 並列実行 + 計画承認フロー
- **P2** (品質+UX): テスト検証 + PR作成 + 失敗ハンドリング + 永続化 + クリーンアップ + ドライラン + 介入 + 報告 + コスト + 中断 + ログ
- **P3**: コンテキスト圧縮

## 既存コード状態

以下のモジュールは基本実装済み（拡張対象）:

- `crates/gwt-core/src/agent/` - mod.rs, master.rs, session.rs, task.rs, sub_agent.rs, conversation.rs, types.rs, worktree.rs
- `crates/gwt-core/src/tmux/` - launcher.rs, pane.rs, poller.rs, detector.rs
- `crates/gwt-core/src/ai/client.rs`
- `crates/gwt-cli/src/tui/screens/agent_mode.rs`

## Commitlintルール

- Conventional Commits形式（`feat:`/`fix:`/`chore:` ...）
- 件名は100文字以内、空にしない
- `bunx commitlint --from HEAD~1 --to HEAD` で自己検証

## Lint最小要件

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --check`
- `cargo test`

---

## フェーズ1: P1 Phase A — Spec Kit内蔵 + UI刷新 + ディープスキャン

**目的**: Spec Kit内蔵化、エージェントモードUIのチャットのみ化、リポジトリスキャナーの実装

### Spec Kit内蔵 (FR-017, FR-018)

- [x] **T001** [P] [共通] `crates/gwt-core/src/speckit/templates/specify.md` に仕様策定プロンプトテンプレートを作成（変数プレースホルダー `{{user_request}}` `{{repository_context}}` 等を含む）
- [x] **T002** [P] [共通] `crates/gwt-core/src/speckit/templates/plan.md` に計画策定プロンプトテンプレートを作成
- [x] **T003** [P] [共通] `crates/gwt-core/src/speckit/templates/tasks.md` にタスク生成プロンプトテンプレートを作成
- [x] **T004** [P] [共通] `crates/gwt-core/src/speckit/templates/clarify.md` に曖昧さ解消プロンプトテンプレートを作成
- [x] **T005** [P] [共通] `crates/gwt-core/src/speckit/templates/analyze.md` に整合性分析プロンプトテンプレートを作成
- [x] **T006** [共通] T001-T005の後に `crates/gwt-core/src/speckit/templates.rs` を新規作成し、`include_str!`マクロで全テンプレートを埋め込み、テンプレート変数置換関数 `render_template(template, vars)` を実装
- [x] **T007** [共通] T006の後に `crates/gwt-core/src/speckit/mod.rs` を新規作成し、SpecKitArtifact構造体の定義とpub modエクスポートを実装
- [x] **T008** [共通] T007の後に `crates/gwt-core/src/speckit/specify.rs` を新規作成し、LLM経由でspec.mdを生成するrun_specify関数を実装
- [x] **T009** [共通] T007の後に `crates/gwt-core/src/speckit/plan.rs` を新規作成し、LLM経由でplan.mdを生成するrun_plan関数を実装
- [x] **T010** [共通] T007の後に `crates/gwt-core/src/speckit/tasks.rs` を新規作成し、LLM経由でtasks.mdを生成するrun_tasks関数を実装
- [x] **T011** [共通] T007の後に `crates/gwt-core/src/speckit/clarify.rs` を新規作成し、LLM経由で質問リストを生成するrun_clarify関数を実装
- [x] **T012** [共通] T007の後に `crates/gwt-core/src/speckit/analyze.rs` を新規作成し、LLM経由で整合性分析を実行するrun_analyze関数を実装
- [x] **T013** [共通] T008-T012の後に `crates/gwt-core/src/lib.rs` に `pub mod speckit;` を追加

### リポジトリディープスキャン (FR-003)

- [x] **T014** [P] [共通] `crates/gwt-core/src/agent/scanner.rs` を新規作成し、RepositoryScanner構造体とRepositoryScanResult構造体を定義。scan関数で `git ls-tree` + CLAUDE.md + Cargo.toml/package.json + specs/ + ソースモジュール概要を収集
- [x] **T015** [共通] T014の後に `crates/gwt-core/src/agent/scanner.rs` にBuildSystem enumを追加し、テストコマンド自動検出（Cargo.toml→`cargo test`、package.json→`npm test`）を実装
- [x] **T016** [共通] T014の後に `crates/gwt-core/src/agent/mod.rs` に `pub mod scanner;` を追加しRepositoryScannerをエクスポート

### アダプティブプロンプト生成 (FR-022)

- [x] **T017** [P] [共通] `crates/gwt-core/src/agent/prompt_builder.rs` を新規作成し、PromptBuilder構造体を定義。build_sub_agent_prompt関数でタスク指示・CLAUDE.md規約・完了指示（`q`終了 / `GWT_TASK_DONE`）を含むプロンプトを生成
- [x] **T018** [共通] T017の後に `crates/gwt-core/src/agent/prompt_builder.rs` にアダプティブ判断ロジックを追加（タスク複雑度に応じてディレクトリ構成・他タスク概要・技術判断結果を含めるか判定）
- [x] **T019** [共通] T017の後に `crates/gwt-core/src/agent/mod.rs` に `pub mod prompt_builder;` を追加しPromptBuilderをエクスポート

### エージェントモードUI刷新 (FR-001, エージェントモードUI仕様)

- [x] **T020** [US1] `crates/gwt-cli/src/tui/screens/agent_mode.rs` からタスクパネル（render_task_panel）を削除し、チャットのみの単一画面に変更。main_chunksの70/30分割レイアウトを削除
- [x] **T021** [US1] T020の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にステータスバーを追加（画面上部にセッション名・キュー待機数・LLMコール数・推定トークン数を表示）
- [x] **T022** [US1] T021の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` のAgentModeStateにsession_name, queue_count, llm_call_count, estimated_tokensフィールドを追加

### データモデル拡張

- [x] **T023** [P] [共通] `crates/gwt-core/src/agent/session.rs` のAgentSessionにbase_branch, spec_id, queue_position, llm_call_count, estimated_tokensフィールドを追加
- [x] **T024** [P] [共通] `crates/gwt-core/src/agent/task.rs` のTaskにtest_status (Option&lt;TestVerification&gt;), retry_count (u8), pull_request (Option&lt;PullRequestRef&gt;) フィールドを追加。TestVerification構造体とTestStatus enumを追加
- [x] **T025** [P] [共通] `crates/gwt-core/src/agent/sub_agent.rs` のSubAgentにauto_mode_flag (Option&lt;String&gt;) フィールドを追加

**✅ Phase A完了チェックポイント**: Spec Kit内蔵・チャットUI・ディープスキャン・プロンプトビルダーが利用可能

---

## フェーズ2: P1 Phase B — タスク分割 + WT作成 + サブエージェント起動 + 完了検出

**目的**: 単一タスクの自律実行フローを確立

### Spec Kit連携ワークフロー (FR-003, FR-003a)

- [x] **T026** [US2] T013の後に `crates/gwt-core/src/agent/master.rs` のMasterAgentにrun_speckit_workflow関数を追加（clarify→specify→plan→tasks の自動フロー）。リポジトリスキャン結果をSpec Kitテンプレートに渡す
- [x] **T027** [US2] T026の後に `crates/gwt-core/src/agent/master.rs` にparse_task_plan関数を追加（LLMレスポンスからJSON形式のタスクリストをパース、失敗時最大2回リトライ）

### Worktree自動作成 (FR-004)

- [x] **T028** [US2] `crates/gwt-core/src/agent/worktree.rs` にsanitize_branch_name関数を追加（英小文字化、空白→ハイフン、記号除去、64文字以内、重複時に連番付与）
- [x] **T029** [US2] T028の後に `crates/gwt-core/src/agent/worktree.rs` にcreate_agent_worktree関数を追加（`agent/`プレフィックス付きブランチ + `.worktrees/` パスで git worktree add）

### サブエージェント起動 (FR-005, 全自動モード)

- [x] **T030** [US3] T025の後に `crates/gwt-core/src/tmux/launcher.rs` にlaunch_auto_mode_agent関数を追加（SubAgentTypeに応じて `--dangerously-skip-permissions` / `--full-auto` 等のフラグをargsに追加してlaunch_agent_in_paneを呼び出し）
- [x] **T031** [US3] T030の後に `crates/gwt-core/src/tmux/pane.rs` にsend_prompt_to_pane関数を追加（長いプロンプトをファイルに書き出し、`tmux load-buffer` + `tmux paste-buffer` でペインに送信）

### 完了検出 (FR-006, FR-007)

- [x] **T032** [P] [US4] `crates/gwt-core/src/tmux/pane.rs` にcapture_pane_output関数を追加（`tmux capture-pane -p -t <pane_id>` で出力を取得）
- [x] **T033** [US4] T032の後に `crates/gwt-core/src/tmux/pane.rs` にdetect_completion_pattern関数を追加（capture-pane出力から `GWT_TASK_DONE` パターンを検出）
- [x] **T034** [US4] T033の後に `crates/gwt-core/src/tmux/pane.rs` にsend_completion_query関数を追加（`tmux send-keys` で状態確認クエリを送信、capture-paneで応答を取得）
- [x] **T035** [US4] T034の後に `crates/gwt-core/src/tmux/poller.rs` のPollMessageにSubAgentCompletedとSubAgentFailedバリアントを追加し、ポーリングループで完了/失敗検出をイベント送信に変換

**✅ Phase B完了チェックポイント**: 単一タスクの自律実行（WT作成→起動→検出）が動作

---

## フェーズ3: P1 Phase C — イベント駆動ループ + 並列実行 + 承認フロー

**目的**: 複数タスクE2E（MVP）

### OrchestratorEvent定義

- [x] **T036** [P] [共通] `crates/gwt-core/src/agent/orchestrator.rs` を新規作成し、OrchestratorEvent enum（SessionStart / UserInput / SubAgentCompleted / SubAgentFailed / TestPassed / TestFailed / ProgressTick / InterruptRequested）を定義
- [x] **T037** [共通] T036の後に `crates/gwt-core/src/agent/mod.rs` に `pub mod orchestrator;` を追加しOrchestratorEventをエクスポート

### イベント駆動ループ (オーケストレーションループ仕様)

- [x] **T038** [US1-4] T037の後に `crates/gwt-core/src/agent/orchestrator.rs` にOrchestratorLoop構造体を追加（mpsc::Sender&lt;OrchestratorEvent&gt; + mpsc::Receiver&lt;OrchestratorEvent&gt; を保持）
- [x] **T039** [US1-4] T038の後に `crates/gwt-core/src/agent/orchestrator.rs` にrun_loop関数を実装（Receiverからイベントを受信→MasterAgentでLLMコール→次アクション決定→実行のループ）
- [x] **T040** [US1-4] T039の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにSessionStartイベントハンドラを追加（Spec Kitワークフロー→タスク生成→承認要求）
- [x] **T041** [US1-4] T040の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにUserInputイベントハンドラを追加（承認応答 / 新規指示 / 質問回答の振り分け）
- [x] **T042** [US1-4] T041の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにSubAgentCompletedイベントハンドラを追加（次のReady状態タスクの起動 or セッション完了判定）

### 承認フロー (FR-002a)

- [x] **T043** [US2] T040の後に `crates/gwt-core/src/agent/orchestrator.rs` にpresent_plan_for_approval関数を追加（spec.md / plan.md / tasks.md の全文をチャットメッセージとして送信）
- [x] **T044** [US2] T043の後に `crates/gwt-core/src/agent/orchestrator.rs` にprocess_approval_response関数を追加（承認→WT作成+起動開始、拒否→再計画）

### 質問フェーズ (ユーザーへの質問UX仕様)

- [x] **T045** [US2] T040の後に `crates/gwt-core/src/agent/orchestrator.rs` にrun_question_phase関数を追加（LLMでclarify+技術質問を統合して生成、デフォルト推奨付与）

### 並列実行制御 (FR-005, サブエージェント並列実行制御仕様)

- [x] **T046** [US3] T042の後に `crates/gwt-core/src/agent/orchestrator.rs` にlaunch_ready_tasks関数を追加（Ready状態のタスクからLLM判断のmax_parallel数まで同時起動）

### 依存関係Git merge (FR-009a)

- [x] **T047** [US3] T046の後に `crates/gwt-core/src/agent/orchestrator.rs` にmerge_dependency_commits関数を追加（先行タスクのコミットを後続ブランチにgit merge）

### TUIイベント連携

- [x] **T048** [US1] T039の後に `crates/gwt-cli/src/tui/app.rs` にOrchestratorLoopのmpsc::Sender&lt;OrchestratorEvent&gt;を保持し、ユーザーのチャット入力をUserInputイベントとして送信するロジックを追加
- [x] **T049** [US1] T048の後に `crates/gwt-cli/src/tui/app.rs` のイベントループにmpsc::Receiver経由でオーケストレーターからのチャットメッセージを受信してAgentModeStateに反映する処理を追加

**✅ MVP達成チェックポイント**: 複数タスクE2Eフロー（入力→質問→承認→並列実行→完了→次タスク）が動作

---

## フェーズ4: P2 — テスト検証 + PR作成 + 失敗ハンドリング (US5, US6)

**目的**: 成果物の品質保証と自律エラー回復

### テスト検証 (FR-008a, 成果物検証仕様)

- [ ] **T050** [US5] T042の後に `crates/gwt-core/src/agent/orchestrator.rs` にrun_test_verification関数を追加（サブエージェントにテストコマンド実行を指示、結果をcapture-paneで取得、パス/失敗でTestPassed/TestFailedイベント送信）
- [ ] **T051** [US5] T050の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにTestPassedイベントハンドラを追加（PR作成フェーズに移行）
- [ ] **T052** [US5] T050の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにTestFailedイベントハンドラを追加（retry_count < 3ならサブエージェントに修正指示、3回目はFailed + ユーザー通知）

### PR作成 (FR-008, PR作成と統合条件仕様)

- [ ] **T053** [P] [US5] `crates/gwt-core/src/agent/orchestrator.rs` にcreate_pull_request関数を追加（git diff取得→LLMでConventional Commits準拠タイトル+詳細本文生成→`gh pr create`実行）
- [ ] **T054** [US5] T053の後に `crates/gwt-core/src/agent/orchestrator.rs` にcheck_pr_prerequisites関数を追加（worktreeクリーン確認、差分存在確認、gh認証確認）

### コンフリクト解決 (FR-009)

- [ ] **T055** [US5] T053の後に `crates/gwt-core/src/agent/orchestrator.rs` にhandle_merge_conflict関数を追加（コンフリクト検出時にサブエージェントにsend-keysで解決指示を送信）

### 失敗ハンドリング (US6)

- [ ] **T056** [US6] T042の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにSubAgentFailedイベントハンドラを追加（LLMで対応策判定: リトライ/代替/ユーザー相談）
- [ ] **T057** [US6] T056の後に `crates/gwt-core/src/agent/orchestrator.rs` にretry_task関数を追加（同一WTでサブエージェント再起動）

### LLM障害時の挙動 (マスターエージェントLLM障害時の挙動仕様)

- [ ] **T058** [P] [US6] `crates/gwt-core/src/ai/client.rs` にエクスポネンシャルバックオフリトライロジックを追加（RateLimited / ServerError時に最大5回リトライ、間隔: 1s→2s→4s→8s→16s）

**✅ P2品質チェックポイント**: テスト検証・PR作成・失敗回復が動作

---

## フェーズ5: P2 — セッション永続化 + クリーンアップ + UX (US7)

**目的**: セッション管理の完成とUX向上

### セッション永続化 (FR-010, FR-011)

- [ ] **T059** [P] [US7] `crates/gwt-core/src/agent/session_store.rs` を新規作成し、SessionStore構造体を定義。sessions_dir関数で `~/.gwt/sessions/` パスを取得、ディレクトリ0700で自動作成
- [ ] **T060** [US7] T059の後に `crates/gwt-core/src/agent/session_store.rs` にsave関数を実装（serde_json::to_string_pretty→一時ファイル書出→atomic rename、パーミッション0600）
- [ ] **T061** [US7] T060の後に `crates/gwt-core/src/agent/session_store.rs` にload関数を実装（JSON読込→AgentSessionへデシリアライズ、破損時は`.broken`にリネームして退避）
- [ ] **T062** [US7] T061の後に `crates/gwt-core/src/agent/session_store.rs` にlist_sessions関数を実装（sessions/ディレクトリの全JSONファイルからSessionId+status+updated_atを一覧取得）
- [ ] **T063** [US7] T062の後に `crates/gwt-core/src/agent/session_store.rs` にvalidate_session関数を実装（各worktreeのパス存在確認、欠落タスクをFailed/Pausedに設定）
- [ ] **T064** [US7] T063の後に `crates/gwt-core/src/agent/mod.rs` に `pub mod session_store;` を追加しSessionStoreをエクスポート

### セッション永続化トリガー

- [ ] **T065** [US7] T064の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loop内の各イベントハンドラ末尾にSessionStore::save呼び出しを追加（会話追加・タスク状態変更・WT作成/削除・サブエージェント状態変更時）

### セッション復元UI

- [ ] **T066** [US7] T064の後に `crates/gwt-cli/src/tui/screens/agent_mode.rs` にrender_session_selector関数を追加（未完了セッション一覧表示、再開/破棄選択UI）
- [ ] **T067** [US7] T066の後に `crates/gwt-cli/src/tui/app.rs` にエージェントモード初期化時にSessionStore::list_sessions呼び出しを追加、未完了セッション存在時にセッション選択画面を表示

### セッションキュー (FR-014)

- [ ] **T068** [P] [共通] `crates/gwt-core/src/agent/orchestrator.rs` にSessionQueue構造体を追加（active: Option&lt;SessionId&gt;, pending: VecDeque&lt;SessionId&gt;）。enqueue / dequeue / current関数を実装

### セッション完了+クリーンアップ (セッション完了とクリーンアップ仕様)

- [ ] **T069** [US7] T065の後に `crates/gwt-core/src/agent/orchestrator.rs` にrun_cleanup関数を追加（全タスクCompleted確認→各WTの未コミット確認→`git worktree remove`→`git branch -d`→リモートブランチ削除→セッションCompleted）
- [ ] **T070** [US7] T069の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにセッション完了判定ロジックを追加（全タスクCompleted + クリーンアップ完了でキュー次セッション開始）

### Esc中断 (FR-024)

- [ ] **T071** [US1] T048の後に `crates/gwt-cli/src/tui/app.rs` のキーハンドラにEscキーでInterruptRequestedイベント送信を追加
- [ ] **T072** [US1] T071の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにInterruptRequestedイベントハンドラを追加（全サブエージェントにSIGTERM→5秒待機→Paused永続化→チャット通知）

### コスト可視化 (FR-015)

- [ ] **T073** [P] [共通] `crates/gwt-core/src/ai/client.rs` のcreate_response戻り値にusage情報（prompt_tokens, completion_tokens）の取得を追加。AIClientにcumulative_tokensフィールドを追加
- [ ] **T074** [共通] T073の後に `crates/gwt-core/src/agent/master.rs` のsend_message内でAIClient::cumulative_tokensをAgentSession::estimated_tokensに反映

### ログ記録 (FR-025)

- [ ] **T075** [P] [共通] `crates/gwt-core/src/agent/master.rs` のsend_message内にtracing::info!マクロでLLMコール記録を追加（カテゴリ: `agent.master.llm`、プロンプト長+レスポンス長を記録）
- [ ] **T076** [共通] T075の後に `crates/gwt-core/src/agent/orchestrator.rs` のサブエージェント起動/完了/失敗箇所にtracing::info!マクロでイベント記録を追加（カテゴリ: `agent.sub`）

### 定期進捗報告 (FR-023)

- [ ] **T077** [共通] T039の後に `crates/gwt-core/src/agent/orchestrator.rs` にspawn_progress_timer関数を追加（2分間隔でProgressTickイベントを送信するスレッド）
- [ ] **T078** [共通] T077の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにProgressTickイベントハンドラを追加（tmux list-panes + capture-paneで各タスク状態・実行時間・最近の出力要約をチャットメッセージとして報告。LLMコールなし）

### ドライランモード (FR-020)

- [ ] **T079** [共通] T043の後に `crates/gwt-core/src/agent/orchestrator.rs` のrun_loopにドライラン判定を追加（ユーザーメッセージに「計画だけ」「dry run」等を検出→仕様+計画+タスクを生成して提示、実行には進まない。後から「実行して」で通常フローに移行）

### ライブ介入 (FR-002c)

- [ ] **T080** [共通] T041の後に `crates/gwt-core/src/agent/orchestrator.rs` のUserInputハンドラにimpact_analysis関数呼び出しを追加（LLMに現在のタスク一覧+新しい指示を渡し、影響タスクを判定→影響タスクのみ停止→再計画）

### セッション継続判断 (FR-021)

- [ ] **T081** [共通] T040の後に `crates/gwt-core/src/agent/orchestrator.rs` のSessionStartハンドラに完了済みセッション関連判定を追加（LLMに既存spec/planと新しいリクエストを渡し、「続き」なら既存セッション拡張、「新規」なら新セッション作成）

**✅ P2完了チェックポイント**: 本番品質のエージェントモード（永続化・中断・復元・コスト・ログ・報告・ドライラン・介入・キュー）

---

## フェーズ6: P2 — ブランチモード連携 + Spec Kitウィザード (FR-019)

**目的**: ブランチモードからのSpec Kit利用とagent/ブランチ表示

### ブランチモード連携 (FR-016)

- [ ] **T082** [P] [共通] `crates/gwt-cli/src/tui/screens/branch_list.rs` でブランチ一覧に`agent/`プレフィックスブランチが表示されることを確認（既存の表示ロジックが対応済みなら変更不要、対応していなければフィルタ解除）

### Spec Kitウィザード (FR-019, ブランチモードからのSpec Kit起動仕様)

- [ ] **T083** [共通] `crates/gwt-cli/src/tui/screens/speckit_wizard.rs` を新規作成し、SpecKitWizardState構造体を定義（input, step: Clarify/Specify/Plan/Tasks/Done, artifacts）
- [ ] **T084** [共通] T083の後に `crates/gwt-cli/src/tui/screens/speckit_wizard.rs` にrender_speckit_wizard関数を実装（ステップ表示 + 機能説明入力フォーム + 進捗表示）
- [ ] **T085** [共通] T084の後に `crates/gwt-cli/src/tui/screens/mod.rs` に `pub mod speckit_wizard;` を追加しエクスポート
- [ ] **T086** [共通] T085の後に `crates/gwt-cli/src/tui/app.rs` にショートカットキーでSpec Kitウィザード起動ロジックを追加（ブランチモード時にキー押下→SpecKitWizardState初期化→ウィザード画面表示→完了後ブランチモードに戻る）

**✅ P2 UX完了チェックポイント**: ブランチモードからSpec Kit利用可能

---

## フェーズ7: P3 — コンテキスト圧縮 (US8)

**目的**: 大規模タスクへの対応

### コンテキスト監視+圧縮 (FR-012)

- [ ] **T087** [US8] `crates/gwt-core/src/agent/master.rs` にestimate_token_count関数を追加（メッセージ配列の文字数からトークン数を推定。英語4文字=1トークン、日本語1文字=1トークンの簡易推定）
- [ ] **T088** [US8] T087の後に `crates/gwt-core/src/agent/master.rs` にshould_compress関数を追加（推定トークン数がmax_contextの80%超過で圧縮判定。max_context取得不可時は16kトークン想定）
- [ ] **T089** [US8] T088の後に `crates/gwt-core/src/agent/master.rs` にcompress_context関数を追加（完了タスク+古い会話をLLMで要約、直近20メッセージは原文保持、要約をSystemメッセージとして挿入）

**✅ 完全な機能**: 全ユーザーストーリー完了

---

## フェーズ8: 統合とポリッシュ

**目的**: 品質保証と最終確認

### 統合テスト

- [ ] **T090** [統合] `cargo test` を実行し全テストパス確認
- [ ] **T091** [統合] `cargo clippy --all-targets --all-features -- -D warnings` を実行し全警告解消
- [ ] **T092** [統合] `cargo fmt --check` を実行しフォーマット統一
- [ ] **T093** [統合] `bunx markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` を実行しMarkdown品質確認

### 成功基準検証

- [ ] **T094** [統合] SC-001検証: Tabキーモード切り替えが1秒以内であることを確認
- [ ] **T095** [統合] SC-002検証: マスターエージェント初回応答が5秒以内であることを確認
- [ ] **T096** [統合] SC-003検証: サブエージェント完了検出が10秒以内であることを確認

### エッジケース対応

- [ ] **T097** [統合] tmux切断時の復旧テスト（セッション永続化→再起動→セッション復元）
- [ ] **T098** [統合] worktree削除時のセッション復元テスト（Failed/Paused状態への遷移確認）

### ドキュメント

- [ ] **T099** [P] [ドキュメント] `README.ja.md` にエージェントモードの使い方セクションを追記
- [ ] **T100** [P] [ドキュメント] `specs/SPEC-ba3f610c/quickstart.md` を最新実装に合わせて更新

---

## タスク凡例

**優先度**:

- **P1 Phase A**: 基盤（Spec Kit + UI + スキャン）— T001-T025
- **P1 Phase B**: コア（タスク分割 + WT + 起動 + 検出）— T026-T035
- **P1 Phase C**: E2E（オーケストレーション + 承認 + 並列）— T036-T049
- **P2 品質**: テスト検証 + PR + 失敗ハンドリング — T050-T058
- **P2 UX**: 永続化 + クリーンアップ + 各種UX — T059-T086
- **P3**: コンテキスト圧縮 — T087-T089
- **統合**: 最終検証 — T090-T100

**ストーリータグ**:

- **[US1]**: モード切り替えと基本対話
- **[US2]**: タスク分割とWorktree自動作成
- **[US3]**: サブエージェント起動と指示
- **[US4]**: サブエージェント完了検出
- **[US5]**: 成果物検証と統合（PR経由）
- **[US6]**: 失敗ハンドリング
- **[US7]**: セッション永続化と再開
- **[US8]**: コンテキスト管理（要約圧縮）
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 並列実行候補

### Phase A内の並列可能グループ

- グループA1: T001, T002, T003, T004, T005（テンプレートファイル作成）
- グループA2: T014, T017（scanner.rs, prompt_builder.rs の新規作成）
- グループA3: T020（agent_mode.rs UI変更）
- グループA4: T023, T024, T025（データモデル拡張、各ファイル独立）

### Phase B内の並列可能グループ

- グループB1: T028, T030, T032（worktree.rs, launcher.rs, pane.rs の各関数追加）

### Phase C内の並列可能グループ

- グループC1: T036（orchestrator.rs新規作成）、T048（app.rsイベント連携）— T036完了後に統合

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化
