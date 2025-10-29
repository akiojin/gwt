export interface WorktreeInfo {
  path: string;
  locked: boolean;
  prunable: boolean;
  isAccessible?: boolean;
}

export interface BranchInfo {
  name: string;
  type: "local" | "remote";
  branchType: "feature" | "hotfix" | "release" | "main" | "develop" | "other";
  isCurrent: boolean;
  description?: string;
  worktree?: WorktreeInfo;
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
  branchType: BranchInfo["branchType"];
  branchDataType: "local" | "remote";
  isCurrent: boolean;
}

export type BranchType =
  | "feature"
  | "hotfix"
  | "release"
  | "main"
  | "develop"
  | "other";

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
  branchTypes: BranchInfo["branchType"][];
  showLocal: boolean;
  showRemote: boolean;
}

export interface PullRequest {
  number: number;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
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
  worktreePath: string | null; // null for local branch only cleanup
  branch: string;
  pullRequest: MergedPullRequest;
  hasUncommittedChanges: boolean;
  hasUnpushedCommits: boolean;
  cleanupType: "worktree-and-branch" | "branch-only";
  hasRemoteBranch?: boolean;
  isAccessible?: boolean;
  invalidReason?: string;
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

// ========================================
// Ink.js UI Types (Phase 2+)
// ========================================

/**
 * Screen types for Ink.js UI
 */
export type ScreenType =
  | "branch-list"
  | "worktree-manager"
  | "branch-creator"
  | "branch-action-selector"
  | "ai-tool-selector"
  | "session-selector"
  | "execution-mode-selector";

/**
 * Branch action types for action selector screen
 */
export type BranchAction = "use-existing" | "create-new";

export type ScreenState = "active" | "hidden";

export interface Screen {
  type: ScreenType;
  state: ScreenState;
  data?: unknown;
}

/**
 * BranchItem - Extended BranchInfo for display purposes
 */
export type WorktreeStatus = "active" | "inaccessible" | undefined;

export interface BranchItem extends BranchInfo {
  // Display properties
  icons: string[];
  worktreeStatus?: WorktreeStatus;
  hasChanges: boolean;
  label: string;
  value: string;
}

/**
 * Statistics - Real-time statistics
 */
export interface Statistics {
  localCount: number;
  remoteCount: number;
  worktreeCount: number;
  changesCount: number;
  lastUpdated: Date;
}

/**
 * Layout - Dynamic layout information
 */
export interface Layout {
  terminalHeight: number;
  terminalWidth: number;
  headerLines: number;
  footerLines: number;
  contentHeight: number;
}
