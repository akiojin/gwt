# タスクリスト: Version History を最新タグ10件に固定表示する（Issue #1230）

## Phase 1: 仕様更新

- [x] T001 [US1,US2] `spec.md` / `plan.md` / `tasks.md` を「最新タグ10件・Unreleased非表示」方針に更新する

## Phase 2: 実装

- [x] T002 [US1] `VersionHistoryPanel.svelte` の一覧取得を `limit: 11` に変更する
- [x] T003 [US1,US2] `VersionHistoryPanel.svelte` で `unreleased` を除外し、先頭10タグのみ表示する
- [x] T004 [US1,US2] `get_project_version_history` 呼び出し対象を表示中タグのみにする
- [x] T005 [US1] `version_history.rs` の `limit=0` 無制限拡張と専用テストを撤回する

## Phase 3: 検証

- [x] T006 [US1,US2] `VersionHistoryPanel.test.ts` を実行して通過を確認する
- [x] T007 [US1] `cargo test -p gwt-tauri list_project_versions_ -- --nocapture` を実行して通過を確認する
