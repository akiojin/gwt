### Core Types

| Name | Kind | Fields | Notes |
|------|------|--------|-------|
| `AssistantToolDefinition` | trait | `name`, `description`, `category`, `permission_level`, `input_schema`, `requires_user_approval` | ツール定義 |
| `AssistantToolHandler` | trait | `execute_async(args) -> ToolResult` | ツール実行ハンドラ |
| `AssistantToolRegistry` | trait | `get_all_tools()`, `get_tools_by_category()`, `get_handler(name)`, `generate_tool_schemas()` | ツールレジストリ |
| `ToolPermissionLevel` | enum | `ReadOnly`, `Write`, `Destructive` | 権限レベル |
| `ToolResult` | struct | `is_error`, `error_code`, `error_message`, `data` | 実行結果（成功/エラー共通） |
| `ToolExecutionLog` | struct | `tool_name`, `args`, `execution_time_ms`, `result_size_bytes`, `timestamp` | 実行ログ |
| `AssistantToolCategory` | enum | `Codebase`, `Git`, `GitHub`, `Agent`, `Pty`, `Spec`, `Session`, `Assistant` | ツールカテゴリ |

### ツール一覧（全カテゴリ）

| カテゴリ | ツール名 | 権限 | 説明 |
|---------|---------|------|------|
| Codebase | `codebase_read_file` | ReadOnly | ファイル内容読み取り |
| Codebase | `codebase_search` | ReadOnly | コード検索（grep） |
| Codebase | `codebase_list_dir` | ReadOnly | ディレクトリ一覧 |
| Codebase | `codebase_get_symbols` | ReadOnly | シンボル一覧（クラス/メソッド） |
| Git | `git_status` | ReadOnly | Git ステータス |
| Git | `git_diff` | ReadOnly | 差分表示 |
| Git | `git_log` | ReadOnly | コミット履歴 |
| Git | `git_worktree_create` | Write | worktree 作成 |
| Git | `git_worktree_remove` | Destructive | worktree 削除（承認必須） |
| Git | `git_worktree_list` | ReadOnly | worktree 一覧 |
| Git | `git_push` | Write | push |
| GitHub | `github_read_issue` | ReadOnly | Issue 読み取り |
| GitHub | `github_create_issue` | Write | Issue 作成 |
| GitHub | `github_update_issue` | Write | Issue 更新 |
| GitHub | `github_list_issues` | ReadOnly | Issue 一覧（ラベルフィルタ対応） |
| GitHub | `github_close_issue` | Destructive | Issue close（承認必須） |
| GitHub | `github_create_pr` | Write | PR 作成 |
| GitHub | `github_get_pr_status` | ReadOnly | PR ステータス（CI 結果含む） |
| GitHub | `github_merge_pr` | Destructive | PR マージ（承認必須） |
| Agent | `agent_hire` | Write | Agent 起動（PTY 起動含む） |
| Agent | `agent_fire` | Write | Agent 停止 |
| Agent | `agent_assign_task` | Write | タスク割当 |
| Agent | `agent_get_status` | ReadOnly | Agent 状態取得 |
| Agent | `agent_list` | ReadOnly | Agent 一覧 |
| PTY | `pty_send_keys` | Write | PTY にキー送信 |
| PTY | `pty_capture_scrollback` | ReadOnly | スクロールバック取得 |
| PTY | `pty_get_output_since` | ReadOnly | 前回以降の出力差分取得 |
| Spec | `spec_start_interview` | Write | インタビューモード開始 |
| Spec | `spec_generate_section` | Write | SPEC セクション生成 |
| Spec | `spec_create_issue` | Write | SPEC → GitHub Issue 作成 |
| Spec | `spec_update_issue` | Write | SPEC Issue 更新 |
| Spec | `spec_check_consistency` | ReadOnly | SPEC 整合性チェック |
| Spec | `spec_list` | ReadOnly | 既存 SPEC 一覧取得 |
| Session | `session_save` | Write | セッション保存 |
| Session | `session_restore` | Write | セッション復元 |
| Session | `session_list` | ReadOnly | セッション一覧 |
| Assistant | `assistant_propose_action` | Write | ユーザーへの提案（承認フロー付き） |
| Assistant | `assistant_ask_question` | Write | ユーザーへの質問 |
| Assistant | `assistant_notify` | Write | 通知表示 |

**合計: 37 ツール（上限 40 以内）**

### Service Boundary

- `AssistantToolRegistry`: ツール定義の一括管理、JSON Schema 生成
- `AssistantToolExecutor`: ツール実行パイプライン（バリデーション → 権限 → 実行 → ログ）
- 各カテゴリハンドラ: `CodebaseToolHandler`, `GitToolHandler`, `GitHubToolHandler`, `AgentToolHandler`, `PtyToolHandler`, `SpecToolHandler`, `SessionToolHandler`, `AssistantActionToolHandler`

---
