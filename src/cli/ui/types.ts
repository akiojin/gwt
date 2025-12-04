import type { LastToolUsage } from "../../types/api.js";
export type { LastToolUsage } from "../../types/api.js";

export interface WorktreeInfo {
  path: string;
  locked: boolean;
  prunable: boolean;
  isAccessible?: boolean;
}

export type AITool = string;
export type InferenceLevel = "low" | "medium" | "high" | "xhigh";

export interface ModelOption {
  id: string;
  label: string;
  description?: string;
  inferenceLevels?: InferenceLevel[];
  defaultInference?: InferenceLevel;
  isDefault?: boolean;
}

export interface BranchDivergence {
  ahead: number;
  behind: number;
  upToDate: boolean;
}

export interface BranchInfo {
  name: string;
  type: "local" | "remote";
  branchType:
    | "feature"
    | "bugfix"
    | "hotfix"
    | "release"
    | "main"
    | "develop"
    | "other";
  isCurrent: boolean;
  description?: string;
  worktree?: WorktreeInfo;
  hasUnpushedCommits?: boolean;
  openPR?: { number: number; title: string };
  mergedPR?: { number: number; mergedAt: string };
  latestCommitTimestamp?: number;
  lastToolUsage?: LastToolUsage | null;
  upstream?: string | null;
  divergence?: BranchDivergence | null;
  hasRemoteCounterpart?: boolean;
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
  | "bugfix"
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

export interface SelectedBranchState {
  name: string;
  displayName: string;
  branchType: "local" | "remote";
  branchCategory: BranchInfo["branchType"];
  remoteBranch?: string;
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
  baseRefName?: string | null;
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

export type CleanupReason = "merged-pr" | "no-diff-with-base" | "remote-synced";

export interface CleanupTarget {
  worktreePath: string | null; // null for local branch only cleanup
  branch: string;
  pullRequest: MergedPullRequest | null;
  hasUncommittedChanges: boolean;
  hasUnpushedCommits: boolean;
  cleanupType: "worktree-and-branch" | "branch-only";
  hasRemoteBranch?: boolean;
  isAccessible?: boolean;
  invalidReason?: string;
  reasons?: CleanupReason[];
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
  baseRefName?: string | null;
}

// ========================================
// Ink.js UI Types (Phase 2+)
// ========================================

/**
 * Screen types for Ink.js UI
 */
export type ScreenType =
  | "branch-list"
  | "branch-creator"
  | "branch-action-selector"
  | "ai-tool-selector"
  | "model-selector"
  | "session-selector"
  | "execution-mode-selector"
  | "batch-merge-progress"
  | "batch-merge-result";

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

export type SyncStatus =
  | "up-to-date"
  | "ahead"
  | "behind"
  | "diverged"
  | "no-upstream"
  | "remote-only";

export interface BranchItem extends BranchInfo {
  // Display properties
  icons: string[];
  worktreeStatus?: WorktreeStatus;
  hasChanges: boolean;
  label: string;
  value: string;
  lastToolUsageLabel?: string | null;
  syncStatus?: SyncStatus;
  syncInfo?: string | undefined;
  remoteName?: string | undefined;
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

// ========================================
// Batch Merge Types (SPEC-ee33ca26)
// ========================================

/**
 * BatchMergeConfig - Configuration for batch merge execution
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export interface BatchMergeConfig {
  sourceBranch: string;
  targetBranches: string[];
  dryRun: boolean;
  autoPush: boolean;
  remote?: string; // default: "origin"
}

/**
 * MergePhase - Current phase of merge operation
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export type MergePhase = "fetch" | "worktree" | "merge" | "push" | "cleanup";

/**
 * BatchMergeProgress - Real-time progress information
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export interface BatchMergeProgress {
  currentBranch: string;
  currentIndex: number;
  totalBranches: number;
  percentage: number; // 0-100
  elapsedSeconds: number;
  estimatedRemainingSeconds?: number;
  currentPhase: MergePhase;
}

/**
 * MergeStatus - Status of individual branch merge
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export type MergeStatus = "success" | "skipped" | "failed";

/**
 * PushStatus - Status of push operation
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export type PushStatus = "success" | "failed" | "not_executed";

/**
 * BranchMergeStatus - Individual branch merge result
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export interface BranchMergeStatus {
  branchName: string;
  status: MergeStatus;
  error?: string;
  conflictFiles?: string[];
  pushStatus?: PushStatus;
  worktreeCreated: boolean;
  durationSeconds: number;
}

/**
 * BatchMergeSummary - Summary statistics
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export interface BatchMergeSummary {
  totalCount: number;
  successCount: number;
  skippedCount: number;
  failedCount: number;
  pushedCount: number;
  pushFailedCount: number;
}

/**
 * BatchMergeResult - Final batch merge result
 * @see specs/SPEC-ee33ca26/data-model.md
 */
export interface BatchMergeResult {
  statuses: BranchMergeStatus[];
  summary: BatchMergeSummary;
  totalDurationSeconds: number;
  cancelled: boolean;
  config: BatchMergeConfig;
}
