# データモデル: エージェントモード

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-01-22

## 概要

エージェントモードで使用するデータ構造を定義する。
すべての構造体はRustで実装し、`serde`でJSON永続化可能とする。

## エンティティ関係図

```text
┌─────────────────────────────────────────────────────────────────┐
│                         AgentSession                            │
│  - id: SessionId                                                │
│  - created_at: DateTime                                         │
│  - updated_at: DateTime                                         │
│  - status: SessionStatus                                        │
├─────────────────────────────────────────────────────────────────┤
│  │                                                              │
│  ├── conversation: Conversation                                 │
│  │   └── messages: Vec<Message>                                 │
│  │                                                              │
│  ├── tasks: Vec<Task>                                           │
│  │   ├── id: TaskId                                             │
│  │   ├── status: TaskStatus                                     │
│  │   ├── dependencies: Vec<TaskId>                              │
│  │   └── sub_agent: Option<SubAgent>                            │
│  │                                                              │
│  └── worktrees: Vec<WorktreeRef>                                │
│      ├── branch_name: String                                    │
│      └── path: PathBuf                                          │
└─────────────────────────────────────────────────────────────────┘
```

## 主要エンティティ

### 1. AgentSession

エージェントモードのセッション全体を表す。

```rust
/// エージェントモードセッション
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    /// 一意のセッションID
    pub id: SessionId,
    /// 作成日時
    pub created_at: DateTime<Utc>,
    /// 最終更新日時
    pub updated_at: DateTime<Utc>,
    /// セッション状態
    pub status: SessionStatus,
    /// ユーザーとマスターエージェントの会話履歴
    pub conversation: Conversation,
    /// 分割されたタスク一覧
    pub tasks: Vec<Task>,
    /// 使用中のworktree参照
    pub worktrees: Vec<WorktreeRef>,
    /// 元のリポジトリパス
    pub repository_path: PathBuf,
}

/// セッションID（UUID形式）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// セッション状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// アクティブ（進行中）
    Active,
    /// 一時停止
    Paused,
    /// 完了
    Completed,
    /// 失敗
    Failed,
}
```

### 2. Conversation

マスターエージェントとユーザーの会話履歴。

```rust
/// 会話履歴
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    /// メッセージ一覧（時系列順）
    pub messages: Vec<Message>,
}

/// 会話メッセージ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// 送信者の役割
    pub role: MessageRole,
    /// メッセージ内容
    pub content: String,
    /// 送信日時
    pub timestamp: DateTime<Utc>,
}

/// メッセージの役割
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    /// ユーザー
    User,
    /// マスターエージェント
    Assistant,
    /// システム（プロンプト等）
    System,
}
```

### 3. Task

マスターエージェントが分割した個別のタスク。

```rust
/// タスク
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// タスクID
    pub id: TaskId,
    /// タスク名（短い説明）
    pub name: String,
    /// タスクの詳細説明
    pub description: String,
    /// タスク状態
    pub status: TaskStatus,
    /// 依存するタスクID一覧
    pub dependencies: Vec<TaskId>,
    /// Worktree戦略
    pub worktree_strategy: WorktreeStrategy,
    /// 割り当てられたworktree
    pub assigned_worktree: Option<WorktreeRef>,
    /// 割り当てられたサブエージェント
    pub sub_agent: Option<SubAgent>,
    /// 作成日時
    pub created_at: DateTime<Utc>,
    /// 開始日時
    pub started_at: Option<DateTime<Utc>>,
    /// 完了日時
    pub completed_at: Option<DateTime<Utc>>,
    /// 結果（完了時）
    pub result: Option<TaskResult>,
}

/// タスクID
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

/// タスク状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// 待機中（依存タスク未完了）
    Pending,
    /// 実行可能（依存タスク完了）
    Ready,
    /// 実行中
    Running,
    /// 完了
    Completed,
    /// 失敗
    Failed,
    /// キャンセル
    Cancelled,
}

/// Worktree戦略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeStrategy {
    /// 新規worktreeを作成
    New,
    /// 既存worktreeを共有（依存タスクと同じ）
    Shared,
}

/// タスク結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// 成功/失敗
    pub success: bool,
    /// 結果の要約
    pub summary: String,
    /// 作成されたPR（あれば）
    pub pull_request: Option<PullRequestRef>,
    /// エラーメッセージ（失敗時）
    pub error: Option<String>,
}

/// PRへの参照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestRef {
    /// PR番号
    pub number: u64,
    /// PR URL
    pub url: String,
}
```

### 4. SubAgent

サブエージェント（Claude Code等）のインスタンス。

```rust
/// サブエージェント
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgent {
    /// サブエージェントID
    pub id: SubAgentId,
    /// エージェント種別
    pub agent_type: SubAgentType,
    /// tmuxペインID
    pub pane_id: String,
    /// プロセスID
    pub pid: u32,
    /// 状態
    pub status: SubAgentStatus,
    /// 起動日時
    pub started_at: DateTime<Utc>,
    /// 完了日時
    pub completed_at: Option<DateTime<Utc>>,
    /// 完了検出方法
    pub completion_source: Option<CompletionSource>,
}

/// サブエージェントID
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubAgentId(pub String);

/// サブエージェント種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentType {
    /// Claude Code
    ClaudeCode,
    /// Codex CLI
    Codex,
    /// Gemini CLI
    Gemini,
    /// OpenCode
    OpenCode,
    /// その他
    Other,
}

/// サブエージェント状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentStatus {
    /// 起動中
    Starting,
    /// 実行中
    Running,
    /// 入力待ち
    WaitingInput,
    /// 完了
    Completed,
    /// エラー
    Error,
}

/// 完了検出方法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionSource {
    /// Claude Code Hook経由
    Hook,
    /// プロセス終了検出
    ProcessExit,
    /// 出力パターン検出
    OutputPattern,
    /// アイドルタイムアウト
    IdleTimeout,
}
```

### 5. WorktreeRef

Worktreeへの参照。

```rust
/// Worktree参照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRef {
    /// ブランチ名（agent/プレフィックス付き）
    pub branch_name: String,
    /// Worktreeのファイルパス
    pub path: PathBuf,
    /// 作成日時
    pub created_at: DateTime<Utc>,
    /// 関連タスクID
    pub task_ids: Vec<TaskId>,
}
```

## 永続化形式

### セッションファイル

**保存場所**: `~/.gwt/sessions/{session_id}.json`

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "created_at": "2026-01-22T10:30:00Z",
  "updated_at": "2026-01-22T11:45:00Z",
  "status": "Active",
  "repository_path": "/path/to/repo",
  "conversation": {
    "messages": [
      {
        "role": "User",
        "content": "認証機能を実装して",
        "timestamp": "2026-01-22T10:30:00Z"
      },
      {
        "role": "Assistant",
        "content": "認証機能を以下のタスクに分割しました...",
        "timestamp": "2026-01-22T10:30:05Z"
      }
    ]
  },
  "tasks": [
    {
      "id": "task-001",
      "name": "JWT認証の実装",
      "description": "...",
      "status": "Completed",
      "dependencies": [],
      "worktree_strategy": "New",
      "assigned_worktree": {
        "branch_name": "agent/jwt-auth",
        "path": "/path/to/.worktrees/agent-jwt-auth",
        "created_at": "2026-01-22T10:31:00Z",
        "task_ids": ["task-001"]
      },
      "sub_agent": {
        "id": "agent-001",
        "agent_type": "ClaudeCode",
        "pane_id": "%5",
        "pid": 12345,
        "status": "Completed",
        "started_at": "2026-01-22T10:31:30Z",
        "completed_at": "2026-01-22T11:00:00Z",
        "completion_source": "Hook"
      },
      "created_at": "2026-01-22T10:30:05Z",
      "started_at": "2026-01-22T10:31:30Z",
      "completed_at": "2026-01-22T11:00:00Z",
      "result": {
        "success": true,
        "summary": "JWT認証を実装しました",
        "pull_request": {
          "number": 123,
          "url": "https://github.com/owner/repo/pull/123"
        },
        "error": null
      }
    }
  ],
  "worktrees": [
    {
      "branch_name": "agent/jwt-auth",
      "path": "/path/to/.worktrees/agent-jwt-auth",
      "created_at": "2026-01-22T10:31:00Z",
      "task_ids": ["task-001"]
    }
  ]
}
```

## 検証ルール

### SessionId

- UUID v4形式
- 空文字列不可

### TaskId

- `task-{number}`形式
- セッション内で一意

### ブランチ名

- `agent/`プレフィックス必須
- 英数字、ハイフン、アンダースコアのみ

### 依存関係

- 循環依存不可
- 存在しないTaskIdへの参照不可

## 状態遷移

### TaskStatus

```text
          ┌───────────┐
          │  Pending  │
          └─────┬─────┘
                │ 依存タスク完了
                ▼
          ┌───────────┐
          │   Ready   │
          └─────┬─────┘
                │ 実行開始
                ▼
          ┌───────────┐
  ┌───────│  Running  │───────┐
  │       └─────┬─────┘       │
  │ エラー      │ 成功        │ キャンセル
  ▼             ▼             ▼
┌──────┐  ┌──────────┐  ┌──────────┐
│Failed│  │Completed │  │Cancelled │
└──────┘  └──────────┘  └──────────┘
```

### SessionStatus

```text
          ┌───────────┐
          │  Active   │◄────────┐
          └─────┬─────┘         │
                │               │ 再開
    ┌───────────┼───────────┐   │
    │ 中断      │ 完了      │   │
    ▼           ▼           │   │
┌──────┐  ┌──────────┐      │   │
│Paused├──┤Completed │      │   │
└──────┘  └──────────┘      │   │
                            │   │
                            ▼   │
                      ┌──────┐  │
                      │Failed├──┘
                      └──────┘
```
