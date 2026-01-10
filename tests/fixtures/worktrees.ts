import type { WorktreeInfo } from "../../src/worktree";

/**
 * テスト用のワークツリーデータ
 */
export const worktrees: WorktreeInfo[] = [
  {
    branch: "main",
    path: "/path/to/repo",
    head: "abc1234",
    isAccessible: true,
  },
  {
    branch: "feature/user-auth",
    path: "/path/to/worktree-feature-user-auth",
    head: "def5678",
    isAccessible: true,
  },
  {
    branch: "feature/dashboard",
    path: "/path/to/worktree-feature-dashboard",
    head: "ghi9012",
    isAccessible: true,
  },
  {
    branch: "hotfix/security-patch",
    path: "/path/to/worktree-hotfix-security-patch",
    head: "jkl3456",
    isAccessible: false, // 異なる環境で作成されたワークツリー
  },
];
