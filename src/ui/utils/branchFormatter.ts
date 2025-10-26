import type {
  BranchInfo,
  BranchItem,
  BranchType,
  WorktreeStatus,
} from "../types.js";
import stringWidth from "string-width";

// Icon mappings
const branchIcons: Record<BranchType, string> = {
  main: "⚡",
  develop: "⚡",
  feature: "✨",
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
};

const remoteIcon = "☁";

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
    const width = stringWidth(icon);
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

  // Column 3: Uncommitted changes / current branch icon
  let changesIcon: string;
  if (branch.isCurrent) {
    changesIcon = padIcon(changeIcons.current);
  } else if (hasChanges) {
    changesIcon = padIcon(changeIcons.hasChanges);
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
  // Format: [Type] [Worktree] [Changes] [Remote] BranchName
  const label = `${branchTypeIcon} ${worktreeIcon} ${changesIcon} ${remoteIconStr} ${branch.name}`;

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
  if (branch.isCurrent) {
    icons.push(changeIcons.current);
  } else if (hasChanges) {
    icons.push(changeIcons.hasChanges);
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
 * Converts an array of BranchInfo to BranchItem array
 */
export function formatBranchItems(branches: BranchInfo[]): BranchItem[] {
  return branches.map((branch) => formatBranchItem(branch));
}
