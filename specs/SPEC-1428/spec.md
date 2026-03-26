### 背景

Windows 環境で Cleanup 機能から Worktree を削除しようとすると、以下のエラーが発生する:

```
[E1013] Git operation failed: worktree remove: fatal: validation failed, cannot remove working tree: '/gwt/.git' does not exist
```

**根本原因:** `git worktree remove` が `.git` ファイル不在で失敗した場合のエラーパターン `"does not exist"` が、フォールバック/リカバリーロジックで認識されていない。

関連 Issue: #1426

### ユーザーシナリオ

| # | シナリオ | 優先度 |
|---|---------|--------|
| US1 | Cleanup で `.git` ファイルが欠損した Worktree を選択→削除実行→正常に削除完了 | P0 |

### 受け入れシナリオ（テストケース）

1. `should_fallback_to_manual_worktree_removal()` が `"does not exist"` を含むエラーメッセージで `true` を返す
2. `is_missing_worktree_error()` が `"does not exist"` を含む `GitOperationFailed` エラーで `true` を返す

### 機能要件

- **FR-001**: `remove_worktree()` は `.git` ファイル不在エラー（`"does not exist"`）時に手動削除 + prune にフォールバックする
- **FR-002**: `cleanup_branch()` は `.git` ファイル不在エラー（`"does not exist"`）時に prune にフォールバックする（安全ネット）

### 成功基準

- **SC-001**: Cleanup 時に `.git` ファイルが欠損した Worktree でもエラーなく削除が完了する
- **SC-002**: 既存のフォールバックパターン（submodule エラー、directory not empty）は影響を受けない
