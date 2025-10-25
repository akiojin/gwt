import type { MergedPullRequest } from '../../src/github';
import type { CleanupTarget } from '../../src/ui/types';

/**
 * テスト用のマージ済みPRデータ
 */
export const mergedPullRequests: MergedPullRequest[] = [
  {
    number: 123,
    title: 'Add user authentication feature',
    headRefName: 'feature/user-auth',
    url: 'https://github.com/test/repo/pull/123',
    state: 'MERGED',
    mergedAt: '2025-01-01T10:00:00Z',
  },
  {
    number: 124,
    title: 'Fix security vulnerability',
    headRefName: 'hotfix/security-patch',
    url: 'https://github.com/test/repo/pull/124',
    state: 'MERGED',
    mergedAt: '2025-01-02T15:30:00Z',
  },
  {
    number: 125,
    title: 'Update dashboard UI',
    headRefName: 'feature/dashboard',
    url: 'https://github.com/test/repo/pull/125',
    state: 'MERGED',
    mergedAt: '2025-01-03T09:15:00Z',
  },
];

/**
 * テスト用のクリーンアップ対象データ
 */
export const cleanupTargets: CleanupTarget[] = [
  {
    branch: 'feature/user-auth',
    worktreePath: '/path/to/worktree-feature-user-auth',
    prNumber: 123,
    prUrl: 'https://github.com/test/repo/pull/123',
    hasUnpushedCommits: false,
    hasRemoteBranch: true,
    cleanupType: 'worktree-and-branch',
  },
  {
    branch: 'hotfix/security-patch',
    worktreePath: undefined, // ワークツリーなし
    prNumber: 124,
    prUrl: 'https://github.com/test/repo/pull/124',
    hasUnpushedCommits: false,
    hasRemoteBranch: true,
    cleanupType: 'branch-only',
  },
  {
    branch: 'feature/dashboard',
    worktreePath: '/path/to/worktree-feature-dashboard',
    prNumber: 125,
    prUrl: 'https://github.com/test/repo/pull/125',
    hasUnpushedCommits: true, // 未プッシュコミットあり
    hasRemoteBranch: false,
    cleanupType: 'worktree-and-branch',
  },
];
