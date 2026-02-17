# TDDノート: GUI Worktree Summary 6タブ + Quick Launchヘッダー再編（Issue #1097）

## 2026-02-17 追記: PR/Workflow 誤表示回帰の修正

### 対象

- `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- `crates/gwt-tauri/src/commands/issue.rs`

### RED

1. ブランチAで `fetch_latest_branch_pr` が in-flight のままブランチBへ切替し、A側がエラーで返ると、B側にも `Failed to load PR` が表示される。
2. `is_issue_not_found_error` が `HTTP 404` を汎用的に許容しており、repo/auth由来の404を「Issue未存在」と誤判定しうる。

### GREEN

1. `loadLatestBranchPr` の catch で branch key 一致時のみ `latestBranchPrError` を反映するよう修正。
2. `is_issue_not_found_error` を issue特化メッセージ（`could not resolve to an issue` / `(repository.issue)`）のみに限定。
3. 回帰テスト追加:
   - `ignores stale latest branch PR errors after branch switch`
   - `test_is_issue_not_found_error` の404誤判定否定ケース

### 実行ログ

- `pnpm -C gwt-gui test src/lib/components/WorktreeSummaryPanel.test.ts` ✅
- `pnpm -C gwt-gui exec playwright test e2e/open-project-smoke.spec.ts` ✅
- `cargo test -p gwt-tauri test_strip_known_remote_prefix_for_origin_and_custom_remote` ✅
- `cargo test -p gwt-tauri test_is_issue_not_found_error` ✅
