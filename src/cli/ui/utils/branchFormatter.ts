import type {
  BranchInfo,
  BranchItem,
  BranchType,
  WorktreeStatus,
  WorktreeInfo,
} from "../types.js";
import stringWidth from "string-width";
import chalk from "chalk";

// Icon mappings
const branchIcons: Record<BranchType, string> = {
  main: "⚡",
  develop: "⚡",
  feature: "✨",
  bugfix: "🐛",
  hotfix: "🔥",
  release: "🚀",
  other: "📌",
};

const worktreeIcons: Record<Exclude<WorktreeStatus, undefined>, string> = {
  active: "🟢",
  inaccessible: "🟠",
};

const changeIcons = {
  current: "👉",
  hasChanges: "💾",
  unpushed: "📤",
  openPR: "🔃",
  mergedPR: "✅",
  warning: "⚠️",
};

const remoteIcon = "☁";

// Sync status icons
const syncIcons = {
  upToDate: "✓",
  ahead: "↑",
  behind: "↓",
  diverged: "↕",
  none: "-",
  remoteOnly: "☁",
};

// Remote column markers
const remoteMarkers = {
  tracked: "🔗", // ローカル+同名リモートあり
  localOnly: "💻", // ローカルのみ（リモートなし）
  remoteOnly: "☁️", // リモートのみ（ローカルなし）
};

// Emoji width varies by terminal. Provide explicit minimum widths so we never
// underestimate and accidentally push the row past the terminal columns.
const iconWidthOverrides: Record<string, number> = {
  // Remote icon
  [remoteIcon]: 1,
  // Branch type icons
  "⚡": 1,
  "✨": 1,
  "🐛": 1,
  "🔥": 1,
  "🚀": 1,
  "📌": 1,
  // Worktree status icons
  "🟢": 1,
  "🟠": 1,
  // Change status icons
  "👉": 1,
  "💾": 1,
  "📤": 1,
  "🔃": 1,
  "✅": 1,
  "⚠️": 1,
  // Remote markers
  "🔗": 1,
  "💻": 1,
  "☁️": 1,
};

const getIconWidth = (icon: string): number => {
  const baseWidth = stringWidth(icon);
  const override = iconWidthOverrides[icon];
  return override !== undefined ? Math.max(baseWidth, override) : baseWidth;
};

export interface FormatOptions {
  hasChanges?: boolean;
}

function mapToolLabel(toolId: string, toolLabel?: string): string {
  if (toolId === "claude-code") return "Claude";
  if (toolId === "codex-cli") return "Codex";
  if (toolId === "gemini-cli") return "Gemini";
  if (toolId === "qwen-cli") return "Qwen";
  if (toolLabel) return toolLabel;
  return "Custom";
}

function mapModeLabel(
  mode?: "normal" | "continue" | "resume" | null,
): string | null {
  if (mode === "normal") return "New";
  if (mode === "continue") return "Continue";
  if (mode === "resume") return "Resume";
  return null;
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
  const modeText = mapModeLabel(usage.mode);
  const timestamp = usage.timestamp ? formatTimestamp(usage.timestamp) : null;
  const parts = [toolText];
  if (modeText) {
    parts.push(modeText);
  }
  if (timestamp) {
    parts.push(timestamp);
  }
  return parts.join(" | ");
}

/**
 * Converts BranchInfo to BranchItem with display properties
 */
export function formatBranchItem(
  branch: BranchInfo,
  options: FormatOptions = {},
): BranchItem {
  const hasChanges = options.hasChanges ?? false;
  const COLUMN_WIDTH = 2; // Fixed width for each icon column

  // Helper to pad icon to fixed width
  const padIcon = (icon: string): string => {
    const width = getIconWidth(icon);
    const padding = Math.max(0, COLUMN_WIDTH - width);
    return icon + " ".repeat(padding);
  };

  // Column 1: Branch type icon (always present)
  const branchTypeIcon = padIcon(branchIcons[branch.branchType]);

  // Column 2: Worktree status icon
  let worktreeStatus: WorktreeStatus;
  let worktreeIcon: string;
  if (branch.worktree) {
    if (branch.worktree.isAccessible === false) {
      worktreeStatus = "inaccessible";
      worktreeIcon = padIcon(worktreeIcons.inaccessible);
    } else {
      worktreeStatus = "active";
      worktreeIcon = padIcon(worktreeIcons.active);
    }
  } else {
    worktreeIcon = " ".repeat(COLUMN_WIDTH);
  }

  // Column 3: Change status icon (priority: ✏️ > ⬆️ > 🔀 > ✅ > ⚠️ > ⭐)
  let changesIcon: string;
  if (hasChanges) {
    changesIcon = padIcon(changeIcons.hasChanges);
  } else if (branch.hasUnpushedCommits) {
    changesIcon = padIcon(changeIcons.unpushed);
  } else if (branch.openPR) {
    changesIcon = padIcon(changeIcons.openPR);
  } else if (branch.mergedPR) {
    changesIcon = padIcon(changeIcons.mergedPR);
  } else if (branch.worktree?.isAccessible === false) {
    changesIcon = padIcon(changeIcons.warning);
  } else if (branch.isCurrent) {
    changesIcon = padIcon(changeIcons.current);
  } else {
    changesIcon = " ".repeat(COLUMN_WIDTH);
  }

  // Column 4: Remote status (✓ for tracked, L for local-only, R for remote-only)
  let remoteStatusStr: string;
  if (branch.type === "remote") {
    // リモートのみのブランチ
    remoteStatusStr = padIcon(remoteMarkers.remoteOnly);
  } else if (branch.hasRemoteCounterpart) {
    // ローカルブランチで同名リモートあり
    remoteStatusStr = padIcon(remoteMarkers.tracked);
  } else {
    // ローカルブランチでリモートなし
    remoteStatusStr = padIcon(remoteMarkers.localOnly);
  }

  // Column 5: Sync status (=, ↑N, ↓N, ↕, -)
  // 5文字固定幅（アイコン1文字 + 数字最大4桁）、9999超は「9999+」表示
  const SYNC_COLUMN_WIDTH = 6; // アイコン1 + 数字4 + スペース1

  // 数字を4桁以内にフォーマット（9999超は9999+）
  const formatSyncNumber = (n: number): string => {
    if (n > 9999) return "9999+";
    return String(n);
  };

  // Sync列を固定幅でパディング
  const padSyncColumn = (content: string): string => {
    const width = stringWidth(content);
    const padding = Math.max(0, SYNC_COLUMN_WIDTH - width);
    return content + " ".repeat(padding);
  };

  let syncStatusStr: string;
  if (branch.type === "remote") {
    // リモートのみ → 比較不可
    syncStatusStr = padSyncColumn(syncIcons.none);
  } else if (branch.divergence) {
    const { ahead, behind, upToDate } = branch.divergence;
    if (upToDate) {
      syncStatusStr = padSyncColumn(syncIcons.upToDate);
    } else if (ahead > 0 && behind > 0) {
      // diverged: ↕ のみ表示（詳細は別途表示可能）
      syncStatusStr = padSyncColumn(syncIcons.diverged);
    } else if (ahead > 0) {
      // ahead: ↑N の形式
      syncStatusStr = padSyncColumn(
        `${syncIcons.ahead}${formatSyncNumber(ahead)}`,
      );
    } else {
      // behind: ↓N の形式
      syncStatusStr = padSyncColumn(
        `${syncIcons.behind}${formatSyncNumber(behind)}`,
      );
    }
  } else {
    // divergence情報なし
    syncStatusStr = padSyncColumn(syncIcons.none);
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

  // Build label with fixed-width columns
  // Format: [Type][Worktree][Changes][Remote][Sync] DisplayName
  const label = `${branchTypeIcon}${worktreeIcon}${changesIcon}${remoteStatusStr}${syncStatusStr}${displayName}`;

  // Collect icons for compatibility
  const icons: string[] = [];
  icons.push(branchIcons[branch.branchType]);
  if (worktreeStatus) {
    icons.push(
      worktreeStatus === "active"
        ? worktreeIcons.active
        : worktreeIcons.inaccessible,
    );
  }
  // Add change icon based on priority
  if (hasChanges) {
    icons.push(changeIcons.hasChanges);
  } else if (branch.hasUnpushedCommits) {
    icons.push(changeIcons.unpushed);
  } else if (branch.openPR) {
    icons.push(changeIcons.openPR);
  } else if (branch.mergedPR) {
    icons.push(changeIcons.mergedPR);
  } else if (branch.worktree?.isAccessible === false) {
    icons.push(changeIcons.warning);
  } else if (branch.isCurrent) {
    icons.push(changeIcons.current);
  }
  if (branch.type === "remote") {
    icons.push(remoteIcon);
  }

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

    // 5. Prioritize most recent commit within same worktree status
    const aCommit = a.latestCommitTimestamp ?? 0;
    const bCommit = b.latestCommitTimestamp ?? 0;
    if (aCommit !== bCommit) {
      return bCommit - aCommit;
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
