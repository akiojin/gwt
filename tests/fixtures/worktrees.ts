import type { WorktreeInfo } from '../../src/worktree';

/**
 * テスト用のワークツリーデータ
 */
export const worktrees: WorktreeInfo[] = [
  {
    branch: 'main',
    path: '/path/to/repo',
    isAccessible: true,
  },
  {
    branch: 'feature/user-auth',
    path: '/path/to/worktree-feature-user-auth',
    isAccessible: true,
  },
  {
    branch: 'feature/dashboard',
    path: '/path/to/worktree-feature-dashboard',
    isAccessible: true,
  },
  {
    branch: 'hotfix/security-patch',
    path: '/path/to/worktree-hotfix-security-patch',
    isAccessible: false, // 異なる環境で作成されたワークツリー
  },
];
