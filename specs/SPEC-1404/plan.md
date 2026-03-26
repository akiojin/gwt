### Part 1: Backend — ブランチ削除ルールの事前検出
- `gh_cli.rs` に `get_branch_deletion_rules()` 追加
- `cleanup.rs` に `get_cleanup_branch_protection` Tauri コマンド追加
- `app.rs` にコマンド登録

### Part 2: Backend — エラーハンドリング（フォールバック）
- `classify_delete_branch_error` に protected ケース追加
- `cleanup_worktrees` の remote 削除結果で "Protected:" をスキップ扱いに

### Part 3: Frontend — 事前表示
- `CleanupModal.svelte` に branchProtection 状態追加
- `getEffectiveSafetyLevel` に保護ブランチ判定追加
- "protected" バッジ表示

### Part 4: テスト
- Rust: classify_delete_branch_error テスト追加
- Frontend: CleanupModal.test.ts テスト追加
