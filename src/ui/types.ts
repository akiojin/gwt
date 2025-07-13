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

export interface PullRequest {
  number: number;
  title: string;
  state: 'OPEN' | 'CLOSED' | 'MERGED';
  branch: string;
  mergedAt: string | null;
  author: string;
}

export interface MergedPullRequest {
  number: number;
  title: string;
  branch: string;
  mergedAt: string;
  author: string;
}

export interface WorktreeWithPR {
  worktreePath: string;
  branch: string;
  pullRequest: PullRequest | null;
}

export interface CleanupTarget {
  worktreePath: string;
  branch: string;
  pullRequest: MergedPullRequest;
  hasUncommittedChanges: boolean;
  hasUnpushedCommits: boolean;
}

export interface GitHubPRAuthor {
  id?: string;
  is_bot?: boolean;
  login?: string;
  name?: string;
}

export interface GitHubPRResponse {
  number: number;
  title: string;
  state: string;
  headRefName: string;
  mergedAt: string | null;
  author: GitHubPRAuthor | null;
}