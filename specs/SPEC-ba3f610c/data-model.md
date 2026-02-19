# データモデル: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-19

## エンティティ関係図

```text
Project (1)
├── Lead (1)                    ← gwt内蔵AI
│   └── Conversation            ← ユーザーとの対話履歴
├── Issue (N)                   ← GitHub Issue対応
│   ├── GitHub Issue (1)        ← 仕様・計画・タスク・TDDを格納
│   ├── Coordinator (1)         ← GUI内蔵ターミナルペイン
│   └── Task (N)                ← GitHub Issueのtasksセクションから生成
│       └── Developer (N)       ← GUI内蔵ターミナルペイン
│           └── Worktree (1)    ← agent/ブランチ
```

## バックエンド（Rust）

### ProjectModeSession

セッション全体のルートエンティティ。`~/.gwt/sessions/{session_id}.json` に永続化。

| フィールド | 型 | 説明 |
|---|---|---|
| id | SessionId | UUID v4 |
| status | SessionStatus | Active / Paused / Completed / Failed |
| created_at | DateTime\<Utc\> | 作成日時 |
| updated_at | DateTime\<Utc\> | 最終更新日時 |
| repository_path | PathBuf | リポジトリルートパス |
| base_branch | String | プロジェクト開始時のブランチ |
| lead | LeadState | Lead状態 |
| issues | Vec\<ProjectIssue\> | Issue一覧 |
| developer_agent_type | AgentType | ユーザー指定のDeveloperエージェント種別 |

### LeadState

| フィールド | 型 | 説明 |
|---|---|---|
| conversation | Vec\<LeadMessage\> | ユーザーとの対話履歴 |
| status | LeadStatus | Idle / Thinking / WaitingApproval / Orchestrating / Error |
| llm_call_count | u64 | LLMコール回数 |
| estimated_tokens | u64 | 推定消費トークン数 |
| active_issue_numbers | Vec\<u64\> | 管理中のGitHub Issue番号一覧 |
| last_poll_at | Option\<DateTime\<Utc\>\> | 最終ポーリング日時 |

### LeadMessage

| フィールド | 型 | 説明 |
|---|---|---|
| role | MessageRole | User / Assistant / System |
| kind | MessageKind | Message / Thought / Action / Observation / Error / Progress |
| content | String | メッセージ本文 |
| timestamp | DateTime\<Utc\> | タイムスタンプ |

### LeadStatus enum

```text
Idle → Thinking → WaitingApproval → Orchestrating → Idle
                → Error → Idle
```

- **Idle**: イベント待ち（LLMコール不要）
- **Thinking**: LLM処理中（ユーザー入力への応答 / GitHub Issue仕様管理実行）
- **WaitingApproval**: ユーザーの承認待ち
- **Orchestrating**: Coordinator管理・進捗監視中
- **Error**: LLM API障害等

### ProjectIssue

| フィールド | 型 | 説明 |
|---|---|---|
| id | String | Issue識別子（例: "issue-10"） |
| github_issue_number | u64 | GitHub Issue番号（必須、仕様管理の一元管理先） |
| github_issue_url | String | GitHub Issue URL |
| title | String | Issueタイトル |
| status | IssueStatus | Pending / Planned / InProgress / CIFail / Completed / Failed |
| coordinator | Option\<CoordinatorState\> | Coordinator状態 |
| tasks | Vec\<ProjectTask\> | タスク一覧 |

### IssueStatus enum

```text
Pending → Planned → InProgress → Completed
                  → CIFail → InProgress (retry)
                  → Failed
```

### CoordinatorState

| フィールド | 型 | 説明 |
|---|---|---|
| pane_id | String | GUI内蔵ターミナルペインID |
| pid | Option\<u32\> | プロセスID |
| status | CoordinatorStatus | Starting / Running / Completed / Crashed / Restarting |
| started_at | DateTime\<Utc\> | 起動日時 |
| github_issue_number | u64 | GitHub Issue番号 |
| crash_count | u8 | クラッシュ回数 |

### CoordinatorStatus enum

```text
Starting → Running → Completed
                   → Crashed → Restarting → Running
```

### ProjectTask

| フィールド | 型 | 説明 |
|---|---|---|
| id | TaskId | UUID v4 |
| name | String | タスク名 |
| description | String | タスク説明 |
| status | TaskStatus | Pending / Ready / Running / Completed / Failed / Cancelled |
| dependencies | Vec\<TaskId\> | 依存タスクID |
| developers | Vec\<DeveloperState\> | 割り当てDeveloper一覧 |
| test_verification | Option\<TestVerification\> | テスト検証結果 |
| pull_request | Option\<PullRequestRef\> | PR情報 |
| retry_count | u8 | リトライ回数 |

### TaskStatus enum

```text
Pending → Ready → Running → Completed
                          → Failed (retry ≤ 3) → Running
                          → Failed (retry > 3) → [terminal]
                → Cancelled
```

### DeveloperState

| フィールド | 型 | 説明 |
|---|---|---|
| id | SubAgentId | UUID v4 |
| agent_type | AgentType | Claude / Codex / Gemini |
| pane_id | String | GUI内蔵ターミナルペインID |
| pid | Option\<u32\> | プロセスID |
| status | DeveloperStatus | Starting / Running / WaitingInput / Completed / Error |
| worktree | WorktreeRef | Worktree情報 |
| started_at | DateTime\<Utc\> | 起動日時 |
| completed_at | Option\<DateTime\<Utc\>\> | 完了日時 |
| completion_source | Option\<CompletionSource\> | 完了検出方法 |

### DeveloperStatus enum

```text
Starting → Running → Completed
                   → Error → [Coordinator判断でリトライ or 差し替え]
```

### WorktreeRef

| フィールド | 型 | 説明 |
|---|---|---|
| branch_name | String | agent/プレフィックス付きブランチ名 |
| path | PathBuf | .worktrees/下のパス |
| created_at | DateTime\<Utc\> | 作成日時 |

### TestVerification

| フィールド | 型 | 説明 |
|---|---|---|
| status | TestStatus | NotRun / Running / Passed / Failed |
| command | String | テストコマンド |
| output | Option\<String\> | テスト出力 |
| attempt | u8 | 試行回数 |

### PullRequestRef

| フィールド | 型 | 説明 |
|---|---|---|
| number | u64 | PR番号 |
| url | String | PR URL |
| ci_status | Option\<CIStatus\> | CI結果 |

### CIStatus enum

Pending / Running / Passed / Failed

### CompletionSource enum

HookStop / ProcessExit / OutputPattern

## フロントエンド（TypeScript）

### ProjectModeState

```typescript
interface ProjectModeState {
  sessionId: string;
  status: "active" | "paused" | "completed" | "failed";
  lead: LeadState;
  issues: ProjectIssue[];
  developerAgentType: "claude" | "codex" | "gemini";
}
```

### LeadState

```typescript
interface LeadState {
  messages: LeadMessage[];
  status: "idle" | "thinking" | "waiting_approval" | "orchestrating" | "error";
  llmCallCount: number;
  estimatedTokens: number;
}
```

### LeadMessage

```typescript
interface LeadMessage {
  role: "user" | "assistant" | "system";
  kind: "message" | "thought" | "action" | "observation" | "error" | "progress";
  content: string;
  timestamp: number;
}
```

### ProjectIssue / DashboardIssue

```typescript
interface ProjectIssue {
  id: string;
  githubIssueNumber: number;
  githubIssueUrl: string;
  title: string;
  status: "pending" | "planned" | "in_progress" | "ci_fail" | "completed" | "failed";
  coordinator?: CoordinatorState;
  tasks: ProjectTask[];
}

type DashboardIssue = ProjectIssue & {
  expanded: boolean;
  taskCompletedCount: number;
  taskTotalCount: number;
};
```

### CoordinatorState

```typescript
interface CoordinatorState {
  paneId: string;
  status: "starting" | "running" | "completed" | "crashed" | "restarting";
}
```

### ProjectTask / DashboardTask

```typescript
interface ProjectTask {
  id: string;
  name: string;
  status: "pending" | "ready" | "running" | "completed" | "failed" | "cancelled";
  developers: DeveloperState[];
  testStatus?: "not_run" | "running" | "passed" | "failed";
  pullRequest?: { number: number; url: string; ciStatus?: string };
  retryCount: number;
}

type DashboardTask = ProjectTask & {
  developerCount: number;
};
```

### DeveloperState

```typescript
interface DeveloperState {
  id: string;
  agentType: "claude" | "codex" | "gemini";
  paneId: string;
  status: "starting" | "running" | "completed" | "error";
  worktree: { branchName: string; path: string };
}
```

## JSON永続化スキーマ

ファイル: `~/.gwt/sessions/{session_id}.json`

```json
{
  "id": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
  "status": "active",
  "created_at": "2026-02-19T10:00:00Z",
  "updated_at": "2026-02-19T12:30:00Z",
  "repository_path": "/path/to/repo",
  "base_branch": "feature/agent-mode",
  "developer_agent_type": "claude",
  "lead": {
    "conversation": [],
    "status": "orchestrating",
    "llm_call_count": 42,
    "estimated_tokens": 150000,
    "active_issue_numbers": [10, 11],
    "last_poll_at": "2026-02-19T12:28:00Z"
  },
  "issues": [
    {
      "id": "issue-10",
      "github_issue_number": 10,
      "github_issue_url": "https://github.com/owner/repo/issues/10",
      "title": "Login feature",
      "status": "in_progress",
      "coordinator": {
        "pane_id": "coord-1",
        "pid": 12345,
        "status": "running",
        "github_issue_number": 10
      },
      "tasks": [
        {
          "id": "task-uuid-1",
          "name": "Implement login form",
          "status": "running",
          "developers": [
            {
              "id": "dev-uuid-1",
              "agent_type": "claude",
              "pane_id": "dev-1",
              "pid": 12346,
              "status": "running",
              "worktree": {
                "branch_name": "agent/login-form",
                "path": ".worktrees/agent-login-form"
              }
            }
          ]
        }
      ]
    }
  ]
}
```

## 既存モデルからの移行

| 既存（gwt-core） | 新規（Project Mode） | 方針 |
|---|---|---|
| AgentSession | ProjectModeSession | フィールド大幅拡張（issues/lead追加） |
| Task | ProjectTask | developers Vec追加（旧sub_agent単体→複数） |
| SubAgent | DeveloperState | リネーム + フィールド整理 |
| WorktreeRef | WorktreeRef | task_ids削除（Developer側から参照） |
| Conversation | LeadState.conversation | kind フィールド追加（progress等） |
| SessionStore | SessionStore | ProjectModeSession対応に拡張 |
| LegacyAgentState | ProjectModeState（フロント） | 3層構造に全面改訂 |
