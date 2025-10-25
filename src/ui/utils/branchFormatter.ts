import type { BranchInfo, BranchItem, BranchType, WorktreeStatus } from '../types.js';

// Icon mappings
const branchIcons: Record<BranchType, string> = {
  main: '⚡',
  develop: '⚡',
  feature: '✨',
  hotfix: '🔥',
  release: '🚀',
  other: '📌',
};

const worktreeIcons: Record<Exclude<WorktreeStatus, undefined>, string> = {
  active: '🟢',
  inaccessible: '🟠',
};

const changeIcons = {
  current: '⭐',
  hasChanges: '✏️',
};

const remoteIcon = '☁';

export interface FormatOptions {
  hasChanges?: boolean;
}

/**
 * Converts BranchInfo to BranchItem with display properties
 */
export function formatBranchItem(
  branch: BranchInfo,
  options: FormatOptions = {}
): BranchItem {
  const icons: string[] = [];
  const hasChanges = options.hasChanges ?? false;

  // Branch type icon
  icons.push(branchIcons[branch.branchType]);

  // Worktree status icon
  let worktreeStatus: WorktreeStatus;
  if (branch.worktree) {
    worktreeStatus = 'active';
    icons.push(worktreeIcons.active);
  }

  // Current branch icon or changes icon
  if (branch.isCurrent) {
    icons.push(changeIcons.current);
  } else if (hasChanges) {
    icons.push(changeIcons.hasChanges);
  }

  // Remote icon
  if (branch.type === 'remote') {
    icons.push(remoteIcon);
  }

  // Create label from icons + branch name
  const iconsStr = icons.join(' ');
  const label = `${iconsStr} ${branch.name}`;

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
