### Phase 1: タスク計画エンジン
- [x] T001 [S] [US-1] LeadTaskPlan, LeadPlannedTask データモデル作成
- [x] T002 [S] [US-1] ProjectContext モデル作成
- [x] T003 [S] [US-1] ILeadTaskPlanner インターフェース定義
- [x] T004 [F] [US-1] LeadTaskPlanner 実装（LLM 呼び出し、JSON パース、ID 一意化、デフォルト値設定）
- [x] T005 [F] [US-1] ProcessUserCommandAsync を LLM タスク計画に置き換え
- [ ] T006 [U] [US-1] SPEC Issue生成ツール（Lead専用内蔵ツール、LLM function calling用）実装

### Phase 2: 実行オーケストレーション
- [x] T007 [F] [US-2,US-3] ILeadService に PlanTasksAsync, ApprovePlanAsync, ExecutePlanAsync, CancelPlanAsync 追加
- [x] T008 [F] [US-2] LeadSessionData に ActivePlan, CompletedPlans フィールド追加
- [x] T009 [F] [US-3] BuildProjectContextAsync ヘルパー実装（IGitService 経由）
- [x] T010 [U] [US-3,US-4] ExecutePlanAsync 実装（依存関係グラフ、ワークツリー作成、エージェント雇用）
- [x] T011 [U] [US-5] shared WorktreeStrategy の再利用ロジック
- [x] T012 [U] [US-4] 失敗タスクのカスケード処理
- [ ] T013 [U] [US-3] force push/rebase禁止ガード実装 + テスト

### Phase 3: 進捗監視強化
- [x] T014 [F] [US-6] AgentOutputBuffer 実装（セッション別バッファ、容量制限）
- [x] T015 [U] [US-6] CheckTaskCompletionAsync 実装（LLM 評価 + フォールバック）
- [x] T016 [U] [US-6] モニタリングループで ActivePlan のタスク監視を統合
- [ ] T017 [U] [US-6] PTY出力停止30秒でのwaiting_input推定ロジック実装 + テスト

### Phase 4: マージ管理
- [x] T018 [F] [US-7,US-8] ILeadMergeManager インターフェース定義
- [x] T019 [U] [US-7,US-8] LeadMergeManager 実装（PR 作成、Mergeable 判定、ワークツリー削除）
- [x] T020 [U] [US-7] タスク完了後の自動 PR 作成フロー

### Phase 5: 進捗レポート
- [x] T021 [U] [US-10] LeadProgressSummary モデル作成
- [x] T022 [U] [US-10] GetProgressSummary 実装
- [x] T023 [U] [US-10] OnProgressChanged イベントの自動発火

### インフラ
- [x] T024 [S] [US-1] GwtAgentInstaller に DI 登録追加
- [x] T025 [S] [US-1] LeadOrchestrator コンストラクタ拡張（6 依存注入）

### テスト
- [x] T026 [U] [US-1] Phase 1 テスト: CreatePlan, UniqueIds, DefaultStrategy, Refine, Serialization (5件)
- [x] T027 [U] [US-2,US-3,US-4,US-5] Phase 2 テスト: Worktree作成, Agent雇用, 依存関係, Shared, StatusUpdate, Approve (6件)
- [x] T028 [U] [US-6] Phase 3 テスト: Buffer容量, RecentLines, AgentCompletion, AgentFailure (4件)
- [x] T029 [U] [US-7,US-8] Phase 4 テスト: PR作成, Mergeable判定, Cleanup (3件)
- [x] T030 [U] [US-10] Phase 5 テスト: ProgressSummary, ProgressChanged (2件)
- [ ] T031 [FIN] [US-1] コンパイル確認（Unity Editor 起動時）
- [ ] T032 [FIN] [US-1] EditMode テスト全パス確認
