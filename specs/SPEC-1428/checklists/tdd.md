### テスト 1: repository.rs

```
manual_worktree_removal_fallback_for_does_not_exist_error
- Input: "fatal: validation failed, cannot remove working tree: '/gwt/.git' does not exist"
- Expected: should_fallback_to_manual_worktree_removal() returns true
```

### テスト 2: manager.rs

```
test_is_missing_worktree_error_does_not_exist
- Input: GwtError::GitOperationFailed { operation: "worktree remove", details: "...does not exist" }
- Expected: is_missing_worktree_error() returns true
```
