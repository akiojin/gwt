import stringWidth from "string-width";
import { BranchInfo } from "./types.js";
import { WorktreeInfo } from "../worktree.js";
import { getChangedFilesCount } from "../git.js";

function stripAnsi(value: string): string {
  // eslint-disable-next-line no-control-regex
  return value.replace(/\u001B\[[0-9;]*m/g, "");
}

function formatIconSlot(icon?: string): string {
  return padEndUnicode(icon ?? "", 2);
}

function getWorktreeIcon(worktree?: WorktreeInfo): string | undefined {
  if (!worktree) {
    return undefined;
  }
  if (worktree.isAccessible === false) {
    return "🟠";
  }
  return "🟢";
}

async function getChangeIcon(
  branch: BranchInfo,
  worktree?: WorktreeInfo,
): Promise<string | undefined> {
  if (branch.isCurrent) {
    return "⭐";
  }
  if (!worktree) {
    return undefined;
  }
  if (worktree.isAccessible === false) {
    return "⚠️";
  }
  try {
    const changedFiles = await getChangedFilesCount(worktree.path);
    if (changedFiles > 0) {
      return "✏️";
    }
  } catch {
    return "⚠️";
  }
  return undefined;
}

function normalizeBranchName(branch: BranchInfo): string {
  if (branch.type === "remote") {
    const slashIndex = branch.name.indexOf("/");
    if (slashIndex === -1) {
      return branch.name;
    }
    return branch.name.slice(slashIndex + 1);
  }
  return branch.name;
}

export async function createBranchTable(
  branches: BranchInfo[],
  worktrees: WorktreeInfo[],
): Promise<Array<{ name: string; value: string; description?: string }>> {
  const worktreeMap = new Map(
    worktrees.filter((w) => w.path !== process.cwd()).map((w) => [w.branch, w]),
  );

  const localBaseNames = new Set(
    branches
      .filter((branch) => branch.type === "local")
      .map((branch) => normalizeBranchName(branch)),
  );
  const remoteBaseNames = new Set(
    branches
      .filter((branch) => branch.type === "remote")
      .map((branch) => normalizeBranchName(branch)),
  );

  const filteredBranches = branches.filter((b) => b.name !== "origin");
  const sortedBranches = [...filteredBranches].sort((a, b) => {
    if (a.isCurrent && !b.isCurrent) return -1;
    if (!a.isCurrent && b.isCurrent) return 1;

    const aIsMain = a.branchType === "main";
    const bIsMain = b.branchType === "main";
    if (aIsMain && !bIsMain) return -1;
    if (!aIsMain && bIsMain) return 1;

    const aIsDevelop = a.branchType === "develop";
    const bIsDevelop = b.branchType === "develop";
    if (aIsDevelop && !bIsDevelop) return -1;
    if (!aIsDevelop && bIsDevelop) return 1;

    const aHasWorktree = worktreeMap.has(a.name);
    const bHasWorktree = worktreeMap.has(b.name);
    if (aHasWorktree && !bHasWorktree) return -1;
    if (!aHasWorktree && bHasWorktree) return 1;

    const aIsLocal = a.type === "local";
    const bIsLocal = b.type === "local";
    if (aIsLocal && !bIsLocal) return -1;
    if (!aIsLocal && bIsLocal) return 1;

    return normalizeBranchName(a).localeCompare(normalizeBranchName(b));
  });

  const lines: Array<{
    raw: string;
    colored: string;
    value: string;
    description?: string;
  }> = [];

  for (const branch of sortedBranches) {
    const normalizedName = normalizeBranchName(branch);

    const hasLocal = localBaseNames.has(normalizedName);
    const hasRemote = remoteBaseNames.has(normalizedName);

    if (branch.type === "remote" && hasLocal) {
      continue;
    }

    const worktree = worktreeMap.get(branch.name);
    const worktreeIcon = getWorktreeIcon(worktree);
    const changeIcon = await getChangeIcon(branch, worktree);

    const branchIcon = getBranchTypeIcon(branch, branch.branchType);
    const iconCluster =
      `${formatIconSlot(branchIcon)}` +
      `${formatIconSlot(worktreeIcon)}` +
      `${formatIconSlot(changeIcon)}`;

    const locationSegment = hasRemote && !hasLocal ? "☁ " : "  ";
    const displayName = branch.type === "remote" ? normalizedName : branch.name;
    const baseLine = `${iconCluster}${locationSegment}${displayName}`;

    lines.push({
      raw: baseLine,
      colored: baseLine,
      value: branch.name,
      description: worktree ? `Worktree: ${worktree.path}` : "No worktree",
    });
  }

  const maxWidth = lines.reduce((max, line) => {
    return Math.max(max, stringWidth(stripAnsi(line.raw)));
  }, 0);

  return lines.map((line) => {
    const padCount = maxWidth - stringWidth(stripAnsi(line.raw));
    const padded = line.raw + " ".repeat(Math.max(0, padCount));
    const entry: { name: string; value: string; description?: string } = {
      name: padded,
      value: line.value,
    };
    if (line.description) {
      entry.description = line.description;
    }
    return entry;
  });
}

function getBranchTypeIcon(
  branch: BranchInfo,
  branchType: BranchInfo["branchType"],
): string {
  switch (branchType) {
    case "main":
      return "⚡";
    case "develop":
      return "⚡";
    case "feature":
      return "✨";
    case "hotfix":
      return "🔥";
    case "release":
      return "🚀";
    default:
      if (branch.type === "remote") {
        return "🌿";
      }
      return "📌";
  }
}

function padEndUnicode(
  str: string,
  targetLength: number,
  padString = " ",
): string {
  const strWidth = stringWidth(str);
  if (strWidth >= targetLength) return str;

  const padWidth = targetLength - strWidth;
  return str + padString.repeat(Math.max(0, padWidth));
}
