### EditMode テスト

#### Phase 1: タスク計画エンジン
- `LeadOrchestrator_CreatePlan_ReturnsStructuredPlan` — LLMレスポンスから構造化計画が生成されること
- `LeadTaskPlanner_EnsureUniqueTaskIds_RenumbersDuplicates` — 重複TaskIdがリナンバーされること
- `LeadTaskPlanner_DefaultStrategy_SetsNewWhenMissing` — WorktreeStrategy未指定時にnewが設定されること
- `LeadTaskPlanner_RefinePlan_PreservesPlanId` — フィードバック後もPlanIdが維持されること
- `LeadTaskPlan_Serialization_RoundTrip` — LeadTaskPlanのシリアライズ/デシリアライズが可逆であること

#### Phase 2: 実行オーケストレーション
- `LeadOrchestrator_ExecutePlan_CreatesWorktreePerTask` — タスクごとにワークツリーが作成されること
- `LeadOrchestrator_ExecutePlan_HiresAgentPerTask` — タスクごとにエージェントが雇用されること
- `LeadOrchestrator_ExecutePlan_RespectsDependencyOrder` — 依存タスクが先行タスク完了後に開始されること
- `LeadOrchestrator_ExecutePlan_SharedWorktree_ReusesExisting` — shared指定タスクが同一ワークツリーを共有すること
- `LeadOrchestrator_ExecutePlan_SetsRunningStatus` — タスク開始時にrunningステータスが設定されること
- `LeadOrchestrator_ApprovePlan_TransitionsDraftToApproved` — 承認操作でdraft→approvedに遷移すること

#### Phase 3: 進捗監視強化
- `AgentOutputBuffer_ExceedsCapacity_DropsOldest` — バッファ容量超過時に古い行が破棄されること
- `AgentOutputBuffer_GetRecentLines_ReturnsLatest` — 最新N行が正しく取得されること
- `LeadOrchestrator_CheckCompletion_DetectsAgentDone` — エージェント完了が正しく検知されること
- `LeadOrchestrator_CheckCompletion_DetectsAgentFailure` — エージェント失敗が正しく検知されること

#### Phase 4: マージ管理
- `LeadMergeManager_CreateTaskPr_SetsTitle` — PRタイトルに`[Lead] {task.Title}`が設定されること
- `LeadMergeManager_TryMerge_ReturnsFalseWhenNotMergeable` — Mergeableでない場合にfalseが返ること
- `LeadMergeManager_Cleanup_DeletesWorktree` — クリーンアップでワークツリーが削除されること

#### Phase 5: 進捗レポート
- `LeadOrchestrator_GetProgressSummary_MatchesTaskStates` — 進捗サマリーがタスク状態と一致すること
- `LeadOrchestrator_OnProgressChanged_FiresOnStatusChange` — ステータス変更時にイベントが発火すること

### PlayMode テスト

- 計画→実行→監視→PR作成の一連フロー（統合テスト）
- マージコンフリクト時のLeadQuestion生成
