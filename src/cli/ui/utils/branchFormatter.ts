import type {
  BranchInfo,
  BranchItem,
  WorktreeStatus,
  WorktreeInfo,
} from "../types.js";

export interface FormatOptions {
  hasChanges?: boolean;
}

function mapToolLabel(toolId: string, toolLabel?: string): string {
  if (toolId === "claude-code") return "Claude";
  if (toolId === "codex-cli") return "Codex";
  if (toolId === "gemini-cli") return "Gemini";
  if (toolLabel) return toolLabel;
  return "Custom";
}

function formatTimestamp(ts: number): string {
  const date = new Date(ts);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}`;
}

function buildLastToolUsageLabel(
  usage?: BranchInfo["lastToolUsage"] | null,
): string | null {
  if (!usage) return null;
  const toolText = mapToolLabel(usage.toolId, usage.toolLabel);
  const timestamp = usage.timestamp ? formatTimestamp(usage.timestamp) : null;
  const parts = [toolText];
  if (timestamp) {
    parts.push(timestamp);
  }
  return parts.join(" | ");
}

/**
 * Calculate the latest activity timestamp for a branch.
 * Returns the maximum of git commit timestamp and tool usage timestamp (in seconds).
 */
export function getLatestActivityTimestamp(branch: BranchInfo): number {
  const gitTimestampSec = branch.latestCommitTimestamp ?? 0;
  // lastToolUsage.timestamp is in milliseconds, convert to seconds
  const toolTimestampSec = branch.lastToolUsage?.timestamp
    ? Math.floor(branch.lastToolUsage.timestamp / 1000)
    : 0;
  return Math.max(gitTimestampSec, toolTimestampSec);
}

/**
 * Converts BranchInfo to BranchItem with display properties
 */
export function formatBranchItem(
  branch: BranchInfo,
  options: FormatOptions = {},
): BranchItem {
  const hasChanges = options.hasChanges ?? false;
  let worktreeStatus: WorktreeStatus | undefined;
  if (branch.worktree) {
    worktreeStatus =
      branch.worktree.isAccessible === false ? "inaccessible" : "active";
  }

  // Build Local/Remote name for display
  // ローカルブランチ: ブランチ名を表示
  // リモートのみ: origin/xxxをフル表示
  let displayName: string;
  let remoteName: string | null = null;

  if (branch.type === "remote") {
    // リモートのみのブランチ: フルのリモートブランチ名を表示
    displayName = branch.name; // origin/xxx
    remoteName = branch.name;
  } else {
    // ローカルブランチ: ブランチ名を表示
    displayName = branch.name;
  }

  const label = displayName;
  const icons: string[] = [];

  // Determine sync status for BranchItem
  let syncStatus: BranchItem["syncStatus"];
  if (branch.type === "remote") {
    syncStatus = "remote-only";
  } else if (!branch.hasRemoteCounterpart) {
    syncStatus = "no-upstream";
  } else if (branch.divergence) {
    const { ahead, behind, upToDate } = branch.divergence;
    if (upToDate) {
      syncStatus = "up-to-date";
    } else if (ahead > 0 && behind > 0) {
      syncStatus = "diverged";
    } else if (ahead > 0) {
      syncStatus = "ahead";
    } else {
      syncStatus = "behind";
    }
  } else {
    syncStatus = "no-upstream";
  }

  return {
    // Copy all properties from BranchInfo
    ...branch,
    // Add display properties
    icons,
    worktreeStatus,
    hasChanges,
    label,
    value: branch.name,
    lastToolUsageLabel: buildLastToolUsageLabel(branch.lastToolUsage),
    syncStatus,
    syncInfo: undefined,
    remoteName: remoteName ?? undefined,
  };
}

/**
 * Sorts branches according to the priority rules:
 * 1. Current branch (highest priority)
 * 2. main branch
 * 3. develop branch (only if main exists)
 * 4. Branches with worktree
 * 5. Latest commit timestamp (descending) within same worktree status
 * 6. Local branches
 * 7. Alphabetical order by name
 */
function sortBranches(
  branches: BranchInfo[],
  worktreeMap: Map<string, WorktreeInfo>,
): BranchInfo[] {
  // Check if main branch exists
  const hasMainBranch = branches.some((b) => b.branchType === "main");

  return [...branches].sort((a, b) => {
    // 1. Current branch is highest priority
    if (a.isCurrent && !b.isCurrent) return -1;
    if (!a.isCurrent && b.isCurrent) return 1;

    // 2. main branch is second priority
    if (a.branchType === "main" && b.branchType !== "main") return -1;
    if (a.branchType !== "main" && b.branchType === "main") return 1;

    // 3. develop branch is third priority (only if main exists)
    if (hasMainBranch) {
      if (a.branchType === "develop" && b.branchType !== "develop") return -1;
      if (a.branchType !== "develop" && b.branchType === "develop") return 1;
    }

    // 4. Branches with worktree are prioritized
    const aHasWorktree = worktreeMap.has(a.name) || !!a.worktree;
    const bHasWorktree = worktreeMap.has(b.name) || !!b.worktree;
    if (aHasWorktree && !bHasWorktree) return -1;
    if (!aHasWorktree && bHasWorktree) return 1;

    // 5. Prioritize most recent activity within same worktree status
    // (max of git commit timestamp and tool usage timestamp)
    const aLatest = getLatestActivityTimestamp(a);
    const bLatest = getLatestActivityTimestamp(b);
    if (aLatest !== bLatest) {
      return bLatest - aLatest;
    }

    // 6. Local branches are prioritized over remote-only
    const aIsLocal = a.type === "local";
    const bIsLocal = b.type === "local";
    if (aIsLocal && !bIsLocal) return -1;
    if (!aIsLocal && bIsLocal) return 1;

    // 7. Alphabetical order by name
    return a.name.localeCompare(b.name);
  });
}

/**
 * Converts an array of BranchInfo to BranchItem array with sorting
 */
export function formatBranchItems(
  branches: BranchInfo[],
  worktreeMap: Map<string, WorktreeInfo> = new Map(),
): BranchItem[] {
  const sortedBranches = sortBranches(branches, worktreeMap);
  return sortedBranches.map((branch) => formatBranchItem(branch));
}
