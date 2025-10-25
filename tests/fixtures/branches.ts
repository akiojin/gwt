import type { BranchInfo } from '../../src/ui/types';

/**
 * テスト用のローカルブランチデータ
 */
export const localBranches: BranchInfo[] = [
  {
    name: 'main',
    type: 'local',
    branchType: 'other',
    isCurrent: true,
  },
  {
    name: 'develop',
    type: 'local',
    branchType: 'other',
    isCurrent: false,
  },
  {
    name: 'feature/user-auth',
    type: 'local',
    branchType: 'feature',
    isCurrent: false,
  },
  {
    name: 'feature/dashboard',
    type: 'local',
    branchType: 'feature',
    isCurrent: false,
  },
  {
    name: 'hotfix/security-patch',
    type: 'local',
    branchType: 'hotfix',
    isCurrent: false,
  },
  {
    name: 'release/1.2.0',
    type: 'local',
    branchType: 'release',
    isCurrent: false,
  },
];

/**
 * テスト用のリモートブランチデータ
 */
export const remoteBranches: BranchInfo[] = [
  {
    name: 'origin/main',
    type: 'remote',
    branchType: 'other',
    isCurrent: false,
  },
  {
    name: 'origin/develop',
    type: 'remote',
    branchType: 'other',
    isCurrent: false,
  },
  {
    name: 'origin/feature/api-integration',
    type: 'remote',
    branchType: 'feature',
    isCurrent: false,
  },
  {
    name: 'origin/hotfix/bug-123',
    type: 'remote',
    branchType: 'hotfix',
    isCurrent: false,
  },
];

/**
 * テスト用の全ブランチデータ
 */
export const allBranches: BranchInfo[] = [...localBranches, ...remoteBranches];
