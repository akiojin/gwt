# タスク一覧: PR Dashboard

**仕様ID**: SPEC-prlist

## Backend タスク

- [x] T001: `gh_cli.rs` に `fetch_pr_list()` 追加
- [x] T002: `gh_cli.rs` に `fetch_authenticated_user()` 追加
- [x] T003: `gh_cli.rs` に `merge_pr()` 追加
- [x] T004: `gh_cli.rs` に `review_pr()` 追加
- [x] T005: `gh_cli.rs` に `mark_pr_ready()` 追加
- [x] T006: `pullrequest.rs` (gwt-core) に `PrListItem` 型追加
- [x] T007: `pullrequest.rs` (gwt-tauri) に `fetch_pr_list` コマンド追加
- [x] T008: `pullrequest.rs` (gwt-tauri) に `fetch_github_user` コマンド追加
- [x] T009: `pullrequest.rs` (gwt-tauri) に `merge_pr` コマンド追加
- [x] T010: `pullrequest.rs` (gwt-tauri) に `review_pr` コマンド追加
- [x] T011: `pullrequest.rs` (gwt-tauri) に `mark_pr_ready` コマンド追加
- [x] T012: `app.rs` にコマンド登録

## Frontend タスク

- [x] T013: `types.ts` に型定義追加
- [x] T014: `menu.rs` にメニュー項目追加
- [x] T015: `App.svelte` にメニューハンドラ追加
- [x] T016: `MainArea.svelte` にタブルーティング追加
- [x] T017: `PrListPanel.svelte` 新規作成
- [x] T018: `MergeDialog.svelte` 新規作成
- [x] T019: `ReviewDialog.svelte` 新規作成

## 検証タスク

- [x] T020: cargo clippy パス
- [x] T021: cargo fmt 確認
- [x] T022: svelte-check パス
- [x] T023: cargo test パス
- [x] T024: pnpm test パス
