import type { BranchInfo, Tab } from "./types";

export function normalizeBranchName(name: string): string {
  const trimmed = name.trim();
  return trimmed.startsWith("origin/")
    ? trimmed.slice("origin/".length)
    : trimmed;
}

export function isRawWorktreeBranch(branchName: string): boolean {
  return ["main", "master", "develop"].includes(normalizeBranchName(branchName));
}

export function resolveWorktreeTabLabel(
  branchName: string | null | undefined,
  branches: BranchInfo[],
  fallback = "Worktree",
): string {
  const normalized = normalizeBranchName(branchName ?? "");
  if (!normalized) return fallback;
  if (isRawWorktreeBranch(normalized)) return normalized;

  const match = branches.find(
    (branch) => normalizeBranchName(branch.name) === normalized,
  );
  const displayName = match?.display_name?.trim() ?? "";
  return displayName || normalized;
}

export function syncAgentTabLabels(tabs: Tab[], branches: BranchInfo[]): Tab[] {
  return tabs.map((tab) => {
    if (tab.type !== "agent") return tab;
    const branchName = tab.branchName?.trim() ?? "";
    if (!branchName) return tab;

    const nextLabel = resolveWorktreeTabLabel(branchName, branches, tab.label);
    if (nextLabel === tab.label) return tab;
    return { ...tab, label: nextLabel };
  });
}

export function findAgentTabByBranchName(
  tabs: Tab[],
  branchName: string,
): Tab | undefined {
  const normalized = normalizeBranchName(branchName);
  return tabs.find(
    (tab) =>
      tab.type === "agent" &&
      normalizeBranchName(tab.branchName ?? "") === normalized,
  );
}
