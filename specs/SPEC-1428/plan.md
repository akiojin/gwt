### 技術コンテキスト

- `crates/gwt-core/src/git/repository.rs`: `should_fallback_to_manual_worktree_removal()` 関数
- `crates/gwt-core/src/worktree/manager.rs`: `is_missing_worktree_error()` メソッド

### 実装アプローチ

両関数のエラーパターンマッチに `"does not exist"` を追加する最小変更。

### フェーズ

1. TDD テスト作成（RED）
2. 本体コード修正（GREEN）
3. 検証
