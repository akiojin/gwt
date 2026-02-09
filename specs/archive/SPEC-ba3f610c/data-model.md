# データモデル設計

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-07

## エンティティ一覧

### Session（セッション）

既存の`AgentSession`を拡張する。

| フィールド | 型 | 説明 |
|-----------|---|------|
| id | SessionId (String) | UUID v4 |
| created_at | DateTime&lt;Utc&gt; | 作成日時 |
| updated_at | DateTime&lt;Utc&gt; | 最終更新日時 |
| status | SessionStatus | Active / Paused / Completed / Failed |
| conversation | Conversation | 対話履歴 |
| tasks | Vec&lt;Task&gt; | タスク一覧 |
| worktrees | Vec&lt;WorktreeRef&gt; | Worktree参照一覧 |
| repository_path | PathBuf | リポジトリルートパス |
| base_branch | String | **【新規】** エージェントモード開始時のブランチ名 |
| spec_id | Option&lt;String&gt; | **【新規】** 関連Spec Kit ID（SPEC-XXXXXXXX） |
| queue_position | Option&lt;usize&gt; | **【新規】** キュー内位置（実行中ならNone） |
| llm_call_count | u32 | **【新規】** LLM APIコール回数 |
| estimated_tokens | u64 | **【新規】** 推定累計トークン数 |

### Task（タスク）

既存の`Task`を拡張する。

| フィールド | 型 | 説明 |
|-----------|---|------|
| id | TaskId (String) | UUID v4 |
| name | String | タスク名 |
| description | String | タスク詳細 |
| status | TaskStatus | Pending / Ready / Running / Completed / Failed / Cancelled |
| dependencies | Vec&lt;TaskId&gt; | 依存タスクID |
| worktree_strategy | WorktreeStrategy | New / Shared |
| assigned_worktree | Option&lt;WorktreeRef&gt; | 割り当てWorktree |
| sub_agent | Option&lt;SubAgent&gt; | 割り当てサブエージェント |
| created_at | DateTime&lt;Utc&gt; | 作成日時 |
| started_at | Option&lt;DateTime&gt; | 開始日時 |
| completed_at | Option&lt;DateTime&gt; | 完了日時 |
| result | Option&lt;TaskResult&gt; | 実行結果 |
| test_status | Option&lt;TestVerification&gt; | **【新規】** テスト検証状態 |
| retry_count | u8 | **【新規】** テスト失敗リトライ回数（最大3） |
| pull_request | Option&lt;PullRequestRef&gt; | **【新規】** 作成されたPR（TaskResultから移動） |

### TestVerification（テスト検証）

**【新規エンティティ】**

| フィールド | 型 | 説明 |
|-----------|---|------|
| status | TestStatus | Pending / Running / Passed / Failed |
| test_command | String | 実行テストコマンド（例: `cargo test`） |
| attempt | u8 | 現在の試行回数 |
| last_output | Option&lt;String&gt; | 最後のテスト出力（要約） |

### SubAgent（サブエージェント）

既存の`SubAgent`を拡張する。

| フィールド | 型 | 説明 |
|-----------|---|------|
| id | SubAgentId (String) | UUID v4 |
| agent_type | SubAgentType | ClaudeCode / Codex / Gemini / OpenCode / Other |
| pane_id | String | tmuxペインID |
| pid | u32 | プロセスID |
| status | SubAgentStatus | Starting / Running / WaitingInput / Completed / Error |
| started_at | DateTime&lt;Utc&gt; | 起動日時 |
| completed_at | Option&lt;DateTime&gt; | 完了日時 |
| completion_source | Option&lt;CompletionSource&gt; | 完了検出方法 |
| auto_mode_flag | Option&lt;String&gt; | **【新規】** 全自動モードフラグ |

### Conversation（対話履歴）

既存の`Conversation`をそのまま使用。P3で要約圧縮を追加。

| フィールド | 型 | 説明 |
|-----------|---|------|
| messages | Vec&lt;Message&gt; | メッセージ配列 |

### Message（メッセージ）

既存の`Message`をそのまま使用。

| フィールド | 型 | 説明 |
|-----------|---|------|
| role | MessageRole | User / Assistant / System |
| content | String | メッセージ内容 |
| timestamp | DateTime&lt;Utc&gt; | タイムスタンプ |

### SessionQueue（セッションキュー）

**【新規エンティティ】** メモリ上のキュー管理構造体。

| フィールド | 型 | 説明 |
|-----------|---|------|
| active | Option&lt;SessionId&gt; | 実行中セッションID |
| pending | VecDeque&lt;SessionId&gt; | 待機中セッションID（FIFO） |

### WorktreeRef（Worktree参照）

既存の`WorktreeRef`をそのまま使用。

| フィールド | 型 | 説明 |
|-----------|---|------|
| branch_name | String | ブランチ名（`agent/`プレフィックス付き） |
| path | PathBuf | Worktreeパス |
| created_at | DateTime&lt;Utc&gt; | 作成日時 |
| task_ids | Vec&lt;TaskId&gt; | 関連タスクID |

### OrchestratorEvent（オーケストレーターイベント）

**【新規エンティティ】** イベント駆動ループのイベント型。

| Variant | データ | 説明 |
|---------|------|------|
| SessionStart | user_message: String | セッション開始（初回入力） |
| UserInput | message: String | ユーザーからのチャット入力 |
| SubAgentCompleted | task_id: TaskId, source: CompletionSource | サブエージェント完了 |
| SubAgentFailed | task_id: TaskId, error: String | サブエージェント失敗 |
| TestPassed | task_id: TaskId | テスト検証パス |
| TestFailed | task_id: TaskId, output: String | テスト検証失敗 |
| ProgressTick | - | 定期進捗報告タイマー（2分） |
| InterruptRequested | - | ユーザーによるEsc中断 |

### SpecKitArtifact（Spec Kit成果物）

**【新規エンティティ】** Spec Kit成果物へのファイル参照。

| フィールド | 型 | 説明 |
|-----------|---|------|
| spec_id | String | SPEC ID（SPEC-XXXXXXXX） |
| spec_path | PathBuf | spec.md パス |
| plan_path | Option&lt;PathBuf&gt; | plan.md パス |
| tasks_path | Option&lt;PathBuf&gt; | tasks.md パス |
| directory | PathBuf | specs/SPEC-XXXXXXXX/ ディレクトリ |

### RepositoryScanResult（リポジトリスキャン結果）

**【新規エンティティ】** セッション開始時のリポジトリスキャンキャッシュ。

| フィールド | 型 | 説明 |
|-----------|---|------|
| claude_md | Option&lt;String&gt; | CLAUDE.md 内容 |
| directory_tree | String | ディレクトリ構造（主要パスのみ） |
| build_system | BuildSystem | Cargo / Npm / Other |
| test_command | String | 推定テストコマンド |
| project_meta | String | Cargo.toml/package.json要約 |
| specs_summary | String | 既存specs/一覧 |
| source_overview | String | モジュール構成概要 |

### BuildSystem（ビルドシステム）

**【新規 enum】**

| Variant | テストコマンド |
|---------|-------------|
| Cargo | `cargo test` |
| Npm | `npm test` |
| Other(String) | ユーザー指定 or 検出不可 |

## 状態遷移図

### TaskStatus 遷移

```text
Pending --> Ready (依存タスクすべてCompleted)
Ready --> Running (サブエージェント起動)
Running --> Completed (テスト検証パス + PR作成完了)
Running --> Failed (テスト3回失敗 / サブエージェントエラー)
Running --> Cancelled (ユーザー中断)
Failed --> Ready (ユーザーがリトライ指示)
```

### SessionStatus 遷移

```text
Active --> Paused (Esc中断 / gwt終了)
Active --> Completed (全タスクCompleted + クリーンアップ完了)
Active --> Failed (回復不能エラー)
Paused --> Active (セッション再開)
```

### SubAgentStatus 遷移

```text
Starting --> Running (プロセス起動確認)
Running --> Completed (完了検出)
Running --> Error (エラー終了)
Running --> WaitingInput (入力待ち検出 - 全自動モードでは発生しにくい)
```

## モジュール間の関係

```text
OrchestratorLoop
  |-- MasterAgent (LLM対話)
  |     |-- AIClient (API通信)
  |     |-- PromptBuilder (プロンプト生成)
  |     +-- Conversation (対話履歴)
  |-- SessionStore (永続化)
  |     +-- AgentSession (セッション状態)
  |-- RepositoryScanner (ディープスキャン)
  |-- SpecKit (仕様策定)
  |     |-- templates (LLMプロンプトテンプレート)
  |     |-- specify / plan / tasks / clarify / analyze
  |     +-- SpecKitArtifact (成果物参照)
  +-- tmux (サブエージェント制御)
        |-- launcher (起動)
        |-- pane (操作・監視)
        +-- poller (イベント検出)
```

## 永続化形式

### セッションファイル

**保存場所**: `~/.gwt/sessions/{session_id}.json`

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "created_at": "2026-02-07T10:30:00Z",
  "updated_at": "2026-02-07T11:45:00Z",
  "status": "Active",
  "repository_path": "/path/to/repo",
  "base_branch": "feature/my-feature",
  "spec_id": "SPEC-ba3f610c",
  "queue_position": null,
  "llm_call_count": 15,
  "estimated_tokens": 24500,
  "conversation": {
    "messages": [
      {
        "role": "User",
        "content": "Add authentication feature",
        "timestamp": "2026-02-07T10:30:00Z"
      },
      {
        "role": "Assistant",
        "content": "I'll break this into the following tasks...",
        "timestamp": "2026-02-07T10:30:05Z"
      }
    ]
  },
  "tasks": [
    {
      "id": "task-001",
      "name": "Implement JWT authentication",
      "description": "...",
      "status": "Completed",
      "dependencies": [],
      "worktree_strategy": "New",
      "assigned_worktree": {
        "branch_name": "agent/jwt-auth",
        "path": "/path/to/.worktrees/agent-jwt-auth",
        "created_at": "2026-02-07T10:31:00Z",
        "task_ids": ["task-001"]
      },
      "sub_agent": {
        "id": "agent-001",
        "agent_type": "ClaudeCode",
        "pane_id": "%5",
        "pid": 12345,
        "status": "Completed",
        "started_at": "2026-02-07T10:31:30Z",
        "completed_at": "2026-02-07T11:00:00Z",
        "completion_source": "Hook",
        "auto_mode_flag": "--dangerously-skip-permissions"
      },
      "created_at": "2026-02-07T10:30:05Z",
      "started_at": "2026-02-07T10:31:30Z",
      "completed_at": "2026-02-07T11:00:00Z",
      "test_status": {
        "status": "Passed",
        "test_command": "cargo test",
        "attempt": 1,
        "last_output": null
      },
      "retry_count": 0,
      "pull_request": {
        "number": 123,
        "url": "https://github.com/owner/repo/pull/123"
      },
      "result": {
        "success": true,
        "summary": "JWT authentication implemented",
        "error": null
      }
    }
  ],
  "worktrees": [
    {
      "branch_name": "agent/jwt-auth",
      "path": "/path/to/.worktrees/agent-jwt-auth",
      "created_at": "2026-02-07T10:31:00Z",
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

- セッション内で一意
- UUID v4形式

### ブランチ名

- `agent/`プレフィックス必須
- 英数字、ハイフン、アンダースコアのみ
- 64文字以内

### 依存関係

- 循環依存不可
- 存在しないTaskIdへの参照不可
