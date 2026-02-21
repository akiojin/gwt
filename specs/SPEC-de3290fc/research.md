# リサーチ: SPEC-de3290fc

## GitHub GraphQL API フィールド調査

### mergeStateStatus (PullRequest)

PullRequest オブジェクトの `mergeStateStatus` フィールドは MergeStateStatus enum を返す。

| 値 | 説明 |
|---|---|
| BEHIND | head ref が base より遅れている |
| BLOCKED | マージがブロックされている |
| CLEAN | マージ可能でコミットステータスも通過 |
| DIRTY | クリーンなマージコミットが作れない |
| DRAFT | ドラフト PR のためマージブロック |
| HAS_HOOKS | マージ可能でステータス通過、pre-receive hooks あり |
| UNKNOWN | 判定不能 |
| UNSTABLE | マージ可能だがコミットステータス未通過 |

Update Branch ボタン表示条件: `mergeStateStatus === "BEHIND"` のときに表示する。

### isRequired (CheckRun)

CheckRun オブジェクトの `isRequired` フィールドは Boolean を返す。

- 引数: `pullRequestId: ID` または `pullRequestNumber: Int`
- Branch Protection Rules の Required Status Checks に含まれるかを判定する
- PR 番号が必要なため、一括クエリ（複数ブランチ）では使えない
- detail クエリ（単一 PR）でのみ使用する

### Update Branch REST API

`PUT /repos/{owner}/{repo}/pulls/{pull_number}/update-branch`

- リクエストボディ: `{ "expected_head_sha": "<sha>" }` (省略可)
- 成功時: `202 Accepted` + `{ "message": "Updating pull request branch.", "url": "..." }`
- gh CLI 経由: `gh api -X PUT /repos/{owner}/{repo}/pulls/{number}/update-branch`

## 既存コードベース調査

### 影響範囲

- `crates/gwt-core/src/git/graphql.rs`: GraphQL クエリビルダーとパーサー
- `crates/gwt-core/src/git/pullrequest.rs`: Rust 型定義
- `crates/gwt-tauri/src/commands/pullrequest.rs`: Tauri コマンド
- `gwt-gui/src/lib/types.ts`: TypeScript 型定義
- `gwt-gui/src/lib/components/PrStatusSection.svelte`: PR 表示コンポーネント
- `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`: タブ構成
- `gwt-gui/src/lib/prStatusHelpers.ts`: Workflow ステータスヘルパー

### isRequired の制約

`isRequired` は PR 番号を引数に取るため、`build_pr_status_query`（複数ブランチ一括）では使用できない。`build_pr_detail_query`（単一 PR 詳細）でのみ PR 番号を渡して使用する。一括クエリで取得する Sidebar のツリー表示では Required 区別は不要（PR タブ内でのみ必要）。
