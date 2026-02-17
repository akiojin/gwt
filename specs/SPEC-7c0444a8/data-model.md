# データモデル: GUI Worktree Summary 7タブ再編（Issue #1097）

## 1. WorktreeSummaryTab

- 種別: UI enum（固定順）
- 値: `quick_start | summary | git | issue | pr | workflow | docker`
- 役割: タブ切替と描画責務の固定化

## 2. BranchLinkedIssue

- `issue_number: number`
- `title: string`
- `state: string`
- `url: string`
- 生成条件: ブランチ名が `issue-<number>` を含む場合のみ

## 3. SelectedBranchPr

- `number: number`
- `state: "open" | "closed" | "merged"`
- `title: string`
- `url: string`
- `check_suites: WorkflowRunInfo[]`
- 選定規則: open 優先、なければ最新 closed/merged

## 4. WorkflowTabState

- `has_pr: boolean`
- `runs: WorkflowRunInfo[]`
- `error: string | null`

## 5. DockerTabState

- `current_context: DockerContext | null`
- `history_rows: ToolSessionEntry[]`
- `error: string | null`
