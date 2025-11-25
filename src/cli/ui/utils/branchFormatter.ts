import type {
  BranchInfo,
  BranchItem,
  BranchType,
  WorktreeStatus,
  WorktreeInfo,
} from "../types.js";
import stringWidth from "string-width";

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
  current: "⭐",
  hasChanges: "✏️",
  unpushed: "⬆️",
  openPR: "🔀",
  mergedPR: "✅",
  warning: "⚠️",
};

const remoteIcon = "☁";

// Some icons are treated as double-width by string-width even though they render
// as a single column in many terminals (e.g. ☁). Provide explicit overrides to
// keep the column layout consistent across environments.
const iconWidthOverrides: Record<string, number> = {
  // Remote icon
  [remoteIcon]: 1,
  "☁️": 1,
  "☁︎": 1,
  // Unpushed icon
  "⬆️": 1,
  "⬆︎": 1,
  "⬆": 1,
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
  "⭐": 1,
  "✏️": 1,
  "🔀": 1,
  "✅": 1,
  "⚠️": 1,
};

export interface FormatOptions {
  hasChanges?: boolean;
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
    const width = iconWidthOverrides[icon] ?? stringWidth(icon);
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

  // Column 4: Remote icon
  let remoteIconStr: string;
  if (branch.type === "remote") {
    remoteIconStr = padIcon(remoteIcon);
  } else {
    remoteIconStr = " ".repeat(COLUMN_WIDTH);
  }

  // Build label with fixed-width columns
  // Format: [Type][Worktree][Changes][Remote] BranchName
  const label = `${branchTypeIcon}${worktreeIcon}${changesIcon}${remoteIconStr}${branch.name}`;

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

  return {
    // Copy all properties from BranchInfo
    ...branch,
    // Add display properties
    icons,
    worktreeStatus,
    hasChanges,
    label,
    value: branch.name,
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
