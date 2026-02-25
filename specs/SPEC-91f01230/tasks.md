# タスクリスト: Version History で古いタグが表示されない（Issue #1230）

## Phase 1: 仕様・テスト準備

- [x] T001 [US1] `specs/SPEC-91f01230/spec.md` / `plan.md` / `tasks.md` を作成して要件を確定する
- [x] T002 [US1] `VersionHistoryPanel.test.ts` を更新し、`list_project_versions` の呼び出しが `limit: 0` であることを検証する
- [x] T003 [US1] `version_history.rs` に `limit=0` 無制限取得のテストを追加する

## Phase 2: 実装

- [x] T004 [US1] `VersionHistoryPanel.svelte` の一覧取得パラメータを `limit: 0` に変更する
- [x] T005 [US1,US2] `list_project_versions` を `limit=0` 無制限・`limit>0` 制限ありで両立するよう修正する

## Phase 3: 検証

- [x] T006 [US1] `VersionHistoryPanel.test.ts` を実行して通過を確認する
- [x] T007 [US1,US2] `crates/gwt-tauri` の `version_history` 関連テストを実行して通過を確認する
