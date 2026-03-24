### Core Types

| Name | Kind | Fields | Notes |
|---|---|---|---|
| `LeadCandidate` | class | `Id`, `DisplayName`, `Personality`, `Description`, `SpriteKey`, `VoiceKey`, `ToneSample` | 雇用候補（口調サンプル付き） |
| `LeadPersonality` | enum | `Professional`, `Friendly`, `Strict` | 性格タイプ |
| `LeadSessionData` | class | `LeadId`, `ProjectRoot`, `ConversationHistory`, `TaskAssignments`, `CurrentState`, `LastMonitoredAt`, `HandoverDocument` | Lead セッション永続化単位 |
| `LeadConversationEntry` | class | `Timestamp`, `Role`, `Content` | user / lead / system 会話 |
| `LeadTaskAssignment` | class | `TaskId`, `IssueNumber`, `AssignedAgentSessionId`, `WorktreePath`, `Branch`, `Status` | Agent への委任単位 |
| `AgentSessionData` | class | `Id`, `AgentType`, `WorktreePath`, `Branch`, `Status`, `PtySessionId`, `ConversationHistory`, `AutoPrCreated` | Lead の監視対象 |
| `HandoverDocument` | class | `Summary`, `SpecStates`, `TaskProgress`, `AgentStatuses`, `OngoingPlans`, `GeneratedAt` | Lead 引継ぎドキュメント（全コンテキスト） |

### State Model

- `CurrentState`: `idle`, `patrolling`, `orchestrating` 系の文字列状態
- `HandoverDocument`: Lead 交代時に全コンテキスト（SPEC 状態、タスク進捗、エージェント状況、進行中の計画）を含む引継ぎ文書
- `TaskAssignments.Status`: `pending`, `in_progress`, `completed`, `failed`

### Service Boundary

- `ILeadService`
  - candidate 選択
  - monitoring start/stop（4秒間隔ポーリング）
  - handover（全コンテキスト含む引継ぎドキュメント生成）
  - user command 処理
  - session save/restore
  - Git 権限スコープ制御（force push / rebase 禁止）
- `IAgentService`
  - session 一覧と状態提供
  - agent 起動/停止/指示送信
