# fix: Cleanup — 保護ブランチの事前表示とエラーハンドリング (#1404)

## 背景

Cleanup処理でリポジトリルールにより保護されたブランチの削除が HTTP 422 でハードエラーになる問題の修正。
事前表示 + エラーハンドリングの2段階で対応。

## 実装ステップ

- [x] T001 gwt-spec Issue 作成 (#1404)
- [x] T002 Rust テスト追加 — classify_delete_branch_error / get_branch_deletion_rules
- [x] T003 `classify_delete_branch_error` に protected ケース追加
- [x] T004 `get_branch_deletion_rules()` 追加
- [x] T005 `get_cleanup_branch_protection` Tauri コマンド追加 + 登録
- [x] T006 `cleanup_worktrees` の "Protected:" ハンドリング
- [x] T007 Frontend テスト追加 — branchProtection / badge (4テスト)
- [x] T008 `CleanupModal.svelte` に保護状態の取得・表示
- [x] T009 全テスト通過確認
- [x] T010 clippy 検証

## 検証結果

- [x] `cargo test -p gwt-core` — 全テスト通過
- [x] `cargo test -p gwt-tauri` — 540テスト通過
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cd gwt-gui && pnpm test src/lib/components/CleanupModal.test.ts` — 39テスト通過
