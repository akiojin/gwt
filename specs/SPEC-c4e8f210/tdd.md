# TDD テスト仕様: Worktree Cleanup（GUI）

## バックエンドテスト（Rust）

### T1: `list_worktrees` コマンド

```rust
// tests/worktree_cleanup_test.rs

#[cfg(test)]
mod list_worktrees_tests {
    // 安全性判定テスト
    #[test]
    fn safe_when_no_changes_and_no_unpushed() {
        // Given: worktree with has_changes=false, has_unpushed=false
        // When: list_worktrees is called
        // Then: safety level is "safe"
    }

    #[test]
    fn warning_when_unpushed_only() {
        // Given: worktree with has_changes=false, has_unpushed=true
        // When: list_worktrees is called
        // Then: safety level is "warning"
    }

    #[test]
    fn warning_when_changes_only() {
        // Given: worktree with has_changes=true, has_unpushed=false
        // When: list_worktrees is called
        // Then: safety level is "warning"
    }

    #[test]
    fn danger_when_both_changes_and_unpushed() {
        // Given: worktree with has_changes=true, has_unpushed=true
        // When: list_worktrees is called
        // Then: safety level is "danger"
    }

    #[test]
    fn protected_branch_is_flagged() {
        // Given: worktree on "main" branch
        // When: list_worktrees is called
        // Then: is_protected=true
    }

    #[test]
    fn current_worktree_is_flagged() {
        // Given: worktree that is currently active
        // When: list_worktrees is called
        // Then: is_current=true
    }

    #[test]
    fn branch_info_fields_are_included() {
        // Given: worktree with ahead=2, behind=1, is_gone=true
        // When: list_worktrees is called
        // Then: WorktreeInfo contains ahead=2, behind=1, is_gone=true
    }

    #[test]
    fn sort_order_is_safety_based() {
        // Given: worktrees with mixed safety levels
        // When: list_worktrees is called
        // Then: results are sorted safe -> warning -> danger -> disabled
    }
}
```

### T2: `cleanup_worktrees` コマンド

```rust
mod cleanup_worktrees_tests {
    #[test]
    fn successful_cleanup_removes_worktree_and_local_branch() {
        // Given: safe worktree on branch "feature/done"
        // When: cleanup_worktrees(["feature/done"], force=false)
        // Then: worktree directory is removed
        // And: local branch "feature/done" is deleted
        // And: result contains {branch: "feature/done", success: true}
    }

    #[test]
    fn does_not_delete_remote_branch() {
        // Given: worktree with remote tracking branch
        // When: cleanup_worktrees(["feature/done"], force=false)
        // Then: remote branch still exists
    }

    #[test]
    fn skips_failure_and_continues() {
        // Given: 3 worktrees, 2nd one is locked
        // When: cleanup_worktrees(["a", "b-locked", "c"], force=false)
        // Then: "a" and "c" are deleted
        // And: "b-locked" result has success=false with error message
    }

    #[test]
    fn refuses_protected_branch_without_force() {
        // Given: worktree on "main"
        // When: cleanup_worktrees(["main"], force=false)
        // Then: result contains {branch: "main", success: false}
    }

    #[test]
    fn refuses_current_worktree() {
        // Given: worktree that is currently active
        // When: cleanup_worktrees([current_branch], force=true)
        // Then: result contains success=false (cannot delete current)
    }

    #[test]
    fn force_deletes_unsafe_worktree() {
        // Given: worktree with uncommitted changes
        // When: cleanup_worktrees(["feature/wip"], force=true)
        // Then: worktree and branch are deleted
    }

    #[test]
    fn emits_progress_events_for_each_branch() {
        // Given: 3 worktrees selected for cleanup
        // When: cleanup_worktrees(["a", "b", "c"], force=false)
        // Then: 3 "cleanup-progress" events are emitted (one per branch)
        // And: 1 "cleanup-completed" event with all results
    }

    #[test]
    fn emits_worktrees_changed_on_completion() {
        // Given: worktrees selected for cleanup
        // When: cleanup_worktrees completes
        // Then: "worktrees-changed" event is emitted
    }
}
```

### T3: `cleanup_single_worktree` コマンド

```rust
mod cleanup_single_worktree_tests {
    #[test]
    fn successful_single_cleanup() {
        // Given: safe worktree on "feature/done"
        // When: cleanup_single_worktree("feature/done", force=false)
        // Then: Ok(())
        // And: worktree and local branch are deleted
    }

    #[test]
    fn fails_on_protected_branch() {
        // Given: worktree on "develop"
        // When: cleanup_single_worktree("develop", force=false)
        // Then: Err with descriptive message
    }

    #[test]
    fn fails_on_unsafe_without_force() {
        // Given: worktree with uncommitted changes
        // When: cleanup_single_worktree("feature/wip", force=false)
        // Then: Err with message about uncommitted changes
    }
}
```

## フロントエンドテスト（TypeScript / Svelte）

### Sidebar 安全性インジケーター

```typescript
// Sidebar.test.ts

describe('Sidebar safety indicators', () => {
    test('displays green dot for safe branches', () => {
        // Given: branch with has_changes=false, has_unpushed=false
        // Then: green CSS dot indicator is rendered
    });

    test('displays yellow dot for warning branches', () => {
        // Given: branch with has_changes=false, has_unpushed=true
        // Then: yellow CSS dot indicator is rendered
    });

    test('displays red dot for danger branches', () => {
        // Given: branch with has_changes=true, has_unpushed=true
        // Then: red CSS dot indicator is rendered
    });

    test('displays gray dot for protected branches', () => {
        // Given: branch with is_protected=true
        // Then: gray CSS dot indicator is rendered
    });

    test('shows spinner for deleting branches', () => {
        // Given: branch in "deleting" state
        // Then: spinner is shown instead of safety dot
        // And: branch row is not clickable
    });

    test('disables click on deleting branches', () => {
        // Given: branch in "deleting" state
        // When: user clicks the branch row
        // Then: no action is taken (click handler not fired)
    });
});
```

### Cleanup モーダル

```typescript
// CleanupModal.test.ts

describe('CleanupModal', () => {
    test('displays all worktrees sorted by safety', () => {
        // Given: worktrees with mixed safety levels
        // When: modal opens
        // Then: items sorted safe -> warning -> danger -> disabled
    });

    test('displays full info per row', () => {
        // Given: worktree with all fields populated
        // Then: row shows safety indicator, branch name, status,
        //       changes, unpushed, ahead/behind, gone, tool
    });

    test('disables checkbox for protected branches', () => {
        // Given: protected branch worktree
        // Then: checkbox is disabled, row appears grayed out
    });

    test('disables checkbox for current worktree', () => {
        // Given: current worktree
        // Then: checkbox is disabled
    });

    test('disables checkbox for agent-running worktree', () => {
        // Given: worktree with running agent
        // Then: checkbox is disabled
    });

    test('Select All Safe checks only safe items', () => {
        // Given: 2 safe, 1 warning, 1 danger, 1 protected worktrees
        // When: "Select All Safe" clicked
        // Then: only 2 safe items are checked
    });

    test('no confirmation when only safe items selected', () => {
        // Given: only safe items checked
        // When: "Cleanup" clicked
        // Then: no confirmation dialog, cleanup starts immediately
    });

    test('shows confirmation when unsafe items selected', () => {
        // Given: warning + danger items checked
        // When: "Cleanup" clicked
        // Then: confirmation dialog appears with count of unsafe items
    });

    test('modal closes immediately after cleanup starts', () => {
        // Given: items selected
        // When: "Cleanup" clicked (and confirmed if needed)
        // Then: modal closes
    });

    test('modal re-opens with errors on partial failure', () => {
        // Given: cleanup completed with 1 failure
        // When: cleanup-completed event received
        // Then: modal re-opens showing failed items with error messages
    });
});
```

### コンテキストメニュー

```typescript
// ContextMenu.test.ts

describe('Branch context menu', () => {
    test('shows both cleanup options', () => {
        // Given: right-click on a branch
        // Then: menu contains "Cleanup this branch" and "Cleanup Worktrees..."
    });

    test('Cleanup this branch shows confirmation dialog', () => {
        // Given: right-click on safe branch
        // When: "Cleanup this branch" selected
        // Then: confirmation dialog appears
    });

    test('Cleanup Worktrees opens modal with preselection', () => {
        // Given: right-click on branch "feature/abc"
        // When: "Cleanup Worktrees..." selected
        // Then: modal opens with "feature/abc" pre-checked
    });

    test('context menu not shown for protected branches single cleanup', () => {
        // Given: right-click on "main" branch
        // Then: "Cleanup this branch" is disabled/not shown
        // And: "Cleanup Worktrees..." is still available
    });
});
```

### キーボードショートカット

```typescript
// Shortcuts.test.ts

describe('Keyboard shortcuts', () => {
    test('Cmd+Shift+K opens cleanup modal', () => {
        // Given: app is focused
        // When: Cmd+Shift+K is pressed
        // Then: Cleanup modal opens
    });
});
```

## テスト実行コマンド

```bash
# バックエンドテスト
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# フロントエンドチェック
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
```
