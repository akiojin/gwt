import type { SessionData } from '../../src/config';

/**
 * テスト用のセッションデータ
 */
export const sessionData: SessionData = {
  lastWorktreePath: '/path/to/worktree-feature-user-auth',
  lastBranch: 'feature/user-auth',
  timestamp: Date.now() - 3600000, // 1時間前
  repositoryRoot: '/path/to/repo',
};

/**
 * テスト用の複数セッションデータ
 */
export const multipleSessionsData: SessionData[] = [
  {
    lastWorktreePath: '/path/to/worktree-feature-user-auth',
    lastBranch: 'feature/user-auth',
    timestamp: Date.now() - 3600000, // 1時間前
    repositoryRoot: '/path/to/repo',
  },
  {
    lastWorktreePath: '/path/to/worktree-feature-dashboard',
    lastBranch: 'feature/dashboard',
    timestamp: Date.now() - 7200000, // 2時間前
    repositoryRoot: '/path/to/repo',
  },
  {
    lastWorktreePath: '/path/to/worktree-hotfix-security-patch',
    lastBranch: 'hotfix/security-patch',
    timestamp: Date.now() - 86400000, // 1日前
    repositoryRoot: '/path/to/repo',
  },
];
