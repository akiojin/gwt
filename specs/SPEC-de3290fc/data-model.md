# データモデル: SPEC-de3290fc

## 1. PrStatusInfo (変更)

既存フィールドに加えて:

- `merge_state_status: Option<String>` — MergeStateStatus enum (BEHIND/BLOCKED/CLEAN/DIRTY/DRAFT/HAS_HOOKS/UNKNOWN/UNSTABLE)

## 2. WorkflowRunInfo (変更)

既存フィールドに加えて:

- `is_required: Option<bool>` — Branch Protection の Required Status Check かどうか

## 3. TypeScript 型 (変更)

### PrStatusInfo

- `mergeStateStatus?: string` — MergeStateStatus ("BEHIND"|"BLOCKED"|"CLEAN"|"DIRTY"|"DRAFT"|"HAS_HOOKS"|"UNKNOWN"|"UNSTABLE"|null)

### WorkflowRunInfo

- `isRequired?: boolean` — Required Status Check かどうか

## 4. 新規 Tauri コマンド

### update_pr_branch

- 入力: `project_path: String`, `pr_number: u64`
- 出力: `Result<String, String>` — 成功メッセージまたはエラー
- 実装: `gh api -X PUT /repos/{owner}/{repo}/pulls/{pr_number}/update-branch`
