### 主要データ型

| 型名 | フィールド | 用途 |
|------|-----------|------|
| `LeadTaskPlan` | PlanId, ProjectRoot, UserRequest, Tasks, CreatedAt, Status | タスク計画の全体管理 |
| `LeadPlannedTask` | TaskId, Title, Description, WorktreeStrategy, SuggestedBranch, AgentType, Instructions, DependsOn, Priority, Status, WorktreePath, Branch, AgentSessionId, PrNumber | 個別タスクの定義と実行状態 |
| `ProjectContext` | ProjectRoot, DefaultBranch, CurrentBranch, AvailableAgents, ExistingBranches | LLM に渡すプロジェクト情報 |
| `LeadProgressSummary` | TotalTasks, CompletedTasks, RunningTasks, FailedTasks, PendingTasks, CreatedPrCount, MergedPrCount | 進捗レポート |

### ステータス遷移

**LeadTaskPlan.Status:**
```
draft → approved → executing → completed / failed
```

**LeadPlannedTask.Status:**
```
pending → running → completed / failed
```

### Persistence
- `LeadSessionData` に `ActivePlan` (LeadTaskPlan) と `CompletedPlans` (List&lt;LeadTaskPlan&gt;) を保持
- JsonUtility でシリアライズ/デシリアライズ（NFR-006）
