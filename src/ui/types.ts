export interface BranchInfo {
  name: string;
  type: 'local' | 'remote';
  branchType: 'feature' | 'hotfix' | 'release' | 'main' | 'develop' | 'other';
  isCurrent: boolean;
  description?: string;
}

export interface BranchChoice {
  name: string;
  value: string;
  description?: string;
  disabled?: boolean | string;
}

export interface EnhancedBranchChoice extends BranchChoice {
  hasWorktree: boolean;
  worktreePath?: string;
  branchType: BranchInfo['branchType'];
  branchDataType: 'local' | 'remote';
  isCurrent: boolean;
}

export type BranchType = 'feature' | 'hotfix' | 'release';

export interface NewBranchConfig {
  type: BranchType;
  taskName: string;
  branchName: string;
}

export interface WorktreeConfig {
  branchName: string;
  worktreePath: string;
  repoRoot: string;
  isNewBranch: boolean;
  baseBranch: string;
}

export interface CleanupResult {
  hasChanges: boolean;
  committed: boolean;
  pushed: boolean;
  worktreeRemoved: boolean;
}

export type ActionType = 'work_existing' | 'create_new' | 'checkout_remote' | 'manage_worktrees';

export interface BranchGroup {
  title: string;
  branches: EnhancedBranchChoice[];
  priority: number;
}

export interface UIFilter {
  showWithWorktree: boolean;
  showWithoutWorktree: boolean;
  branchTypes: BranchInfo['branchType'][];
  showLocal: boolean;
  showRemote: boolean;
}